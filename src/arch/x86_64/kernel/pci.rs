use alloc::fmt::Debug;
use core::iter::IntoIterator;

use pci_types::{ConfigRegionAccess, PciAddress, PciHeader};
use x86_64::instructions::port::Port;

use crate::drivers::pci::{PCI_DEVICES, PciDevice};

const PCI_CONFIG_ADDRESS_ENABLE: u32 = 1 << 31;

const CONFIG_ADDRESS: Port<u32> = Port::new(0xcf8);
const CONFIG_DATA: Port<u32> = Port::new(0xcfc);

#[derive(Debug, Copy, Clone)]
pub enum PciConfigRegion {
	#[allow(private_interfaces)]
	Pci(LegacyPciConfigRegion),
	#[cfg(feature = "acpi")]
	PciE(pcie::McfgEntry),
}

impl ConfigRegionAccess for PciConfigRegion {
	unsafe fn read(&self, address: PciAddress, offset: u16) -> u32 {
		match self {
			PciConfigRegion::Pci(entry) => unsafe { entry.read(address, offset) },
			#[cfg(feature = "acpi")]
			PciConfigRegion::PciE(entry) => unsafe { entry.read(address, offset) },
		}
	}

	unsafe fn write(&self, address: PciAddress, offset: u16, value: u32) {
		match self {
			PciConfigRegion::Pci(entry) => unsafe {
				entry.write(address, offset, value);
			},
			#[cfg(feature = "acpi")]
			PciConfigRegion::PciE(entry) => unsafe {
				entry.write(address, offset, value);
			},
		}
	}
}

#[derive(Debug, Copy, Clone)]
struct LegacyPciConfigRegion;

impl LegacyPciConfigRegion {
	pub const fn new() -> Self {
		Self {}
	}
}

impl ConfigRegionAccess for LegacyPciConfigRegion {
	#[inline]
	unsafe fn read(&self, pci_addr: PciAddress, register: u16) -> u32 {
		let mut config_address = CONFIG_ADDRESS;
		let mut config_data = CONFIG_DATA;

		let address = PCI_CONFIG_ADDRESS_ENABLE
			| (u32::from(pci_addr.bus()) << 16)
			| (u32::from(pci_addr.device()) << 11)
			| (u32::from(pci_addr.function()) << 8)
			| u32::from(register);

		unsafe {
			config_address.write(address);
			config_data.read()
		}
	}

	#[inline]
	unsafe fn write(&self, pci_addr: PciAddress, register: u16, value: u32) {
		let mut config_address = CONFIG_ADDRESS;
		let mut config_data = CONFIG_DATA;

		let address = PCI_CONFIG_ADDRESS_ENABLE
			| (u32::from(pci_addr.bus()) << 16)
			| (u32::from(pci_addr.device()) << 11)
			| (u32::from(pci_addr.function()) << 8)
			| u32::from(register);

		unsafe {
			config_address.write(address);
			config_data.write(value);
		}
	}
}

pub fn init() {
	#[cfg(feature = "acpi")]
	if pcie::init_pcie() {
		info!("Initialized PCIe");
		return;
	}

	// For Hermit, we currently limit scanning to the first 32 buses.
	const PCI_MAX_BUS_NUMBER: u8 = 32;
	scan_bus(
		0..PCI_MAX_BUS_NUMBER,
		PciConfigRegion::Pci(LegacyPciConfigRegion::new()),
	);
	info!("Initialized PCI");
}

fn scan_bus(bus_range: impl IntoIterator<Item = u8> + Debug, pci_config: PciConfigRegion) {
	debug!("Scanning PCI buses {bus_range:?}");

	// Hermit only uses PCI for network devices.
	// Therefore, multifunction devices as well as additional bridges are not scanned.
	for bus in bus_range {
		// For Hermit, we currently limit scanning to the first 32 devices.
		const PCI_MAX_DEVICE_NUMBER: u8 = 32;
		for device in 0..PCI_MAX_DEVICE_NUMBER {
			let pci_address = PciAddress::new(0, bus, device, 0);
			let header = PciHeader::new(pci_address);

			let (device_id, vendor_id) = header.id(pci_config);
			if device_id != u16::MAX && vendor_id != u16::MAX {
				let device = PciDevice::new(pci_address, pci_config);
				PCI_DEVICES.with(|pci_devices| pci_devices.unwrap().push(device));
			}
		}
	}
}

#[cfg(feature = "acpi")]
mod pcie {
	use core::{ptr, slice};

	use pci_types::{ConfigRegionAccess, PciAddress};

	use super::PciConfigRegion;
	use crate::arch::x86_64::kernel::acpi;

	pub fn init_pcie() -> bool {
		let Some(table) = acpi::get_mcfg_table() else {
			return false;
		};

		let start = ptr::with_exposed_provenance::<McfgEntry>(table.table_start_address() + 8);
		let end = ptr::with_exposed_provenance::<McfgEntry>(table.table_end_address());
		let entries = unsafe { slice::from_ptr_range(start..end) };

		if entries.is_empty() {
			return false;
		}

		for entry in entries {
			init_pcie_bus(entry);
		}

		true
	}

	#[derive(Clone, Copy, Debug)]
	#[repr(C, packed)]
	pub struct McfgEntry {
		pub base_address: usize,
		pub pci_segment_group: u16,
		pub bus_number_start: u8,
		pub bus_number_end: u8,
		_reserved: u32,
	}

	impl McfgEntry {
		pub fn pci_config_space_address(
			&self,
			bus_number: u8,
			device: u8,
			function: u8,
		) -> *mut u32 {
			unsafe {
				ptr::with_exposed_provenance_mut::<u32>(self.base_address).byte_add(
					(usize::from(bus_number) << 20)
						| ((usize::from(device) & 0x1f) << 15)
						| ((usize::from(function) & 0x7) << 12),
				)
			}
		}
	}

	impl ConfigRegionAccess for McfgEntry {
		unsafe fn read(&self, address: PciAddress, offset: u16) -> u32 {
			assert!(address.segment() == self.pci_segment_group);
			assert!(address.bus() >= self.bus_number_start);
			assert!(address.bus() <= self.bus_number_end);

			unsafe {
				self.pci_config_space_address(address.bus(), address.device(), address.function())
					.byte_add(usize::from(offset))
					.read_volatile()
			}
		}

		unsafe fn write(&self, address: PciAddress, offset: u16, value: u32) {
			assert!(address.segment() == self.pci_segment_group);
			assert!(address.bus() >= self.bus_number_start);
			assert!(address.bus() <= self.bus_number_end);

			unsafe {
				self.pci_config_space_address(address.bus(), address.device(), address.function())
					.byte_add(usize::from(offset))
					.write_volatile(value);
			}
		}
	}

	fn init_pcie_bus(bus_entry: &McfgEntry) {
		super::scan_bus(
			bus_entry.bus_number_start..=bus_entry.bus_number_end,
			PciConfigRegion::PciE(*bus_entry),
		);
	}
}
