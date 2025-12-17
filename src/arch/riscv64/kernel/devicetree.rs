#![allow(dead_code)]

use memory_addresses::PhysAddr;

use crate::arch::riscv64::kernel::interrupts::init_plic;
use crate::arch::riscv64::mm::paging::{self, PageSize};
use crate::env;

static mut PLATFORM_MODEL: Model = Model::Unknown;

enum Model {
	Fux40,
	Virt,
	Unknown,
}

/// Inits variables based on the device tree
/// This function should only be called once
pub fn init() {
	debug!("Init devicetree");
	if let Some(fdt) = env::fdt() {
		let model = fdt
			.find_node("/")
			.unwrap()
			.property("compatible")
			.expect("compatible not found in FDT")
			.as_str()
			.unwrap();

		let platform_model = if model.contains("riscv-virtio") {
			Model::Virt
		} else if model.contains("sifive,hifive-unmatched-a00")
			|| model.contains("sifive,hifive-unleashed-a00")
			|| model.contains("sifive,fu740")
			|| model.contains("sifive,fu540")
		{
			Model::Fux40
		} else {
			warn!("Unknown platform, guessing PLIC context 1");
			Model::Unknown
		};
		unsafe {
			PLATFORM_MODEL = platform_model;
		}
		info!("Model: {model}");
	}
}

/// Inits drivers based on the device tree
/// This function should only be called once
pub fn init_drivers() {
	// TODO: Implement devicetree correctly
	if let Some(fdt) = env::fdt() {
		debug!("Init drivers using devicetree");

		unsafe {
			// Init PLIC first
			if let Some(plic_node) = fdt.find_compatible(&["sifive,plic-1.0.0"]) {
				debug!("Found interrupt controller");
				let plic_region = plic_node
					.reg()
					.expect("Reg property for PLIC not found in FDT")
					.next()
					.unwrap();

				let plic_region_start = PhysAddr::new(plic_region.starting_address as u64);
				debug!(
					"Init PLIC at {:p}, size: {:x}",
					plic_region_start,
					plic_region.size.unwrap()
				);
				assert!(
					plic_region.size.unwrap()
						< usize::try_from(paging::HugePageSize::SIZE).unwrap()
				);

				paging::identity_map::<paging::HugePageSize>(plic_region_start);

				// TODO: Determine correct context via devicetree and allow more than one context
				match PLATFORM_MODEL {
					Model::Virt | Model::Unknown => {
						init_plic(plic_region.starting_address as usize, 1);
					}
					Model::Fux40 => init_plic(plic_region.starting_address as usize, 2),
				}
			}
		}
	}
}
