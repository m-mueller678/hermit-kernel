//! A module containing hermit-rs driver, hermit-rs driver trait and driver specific errors.

#[cfg(not(feature = "pci"))]
pub mod mmio;
#[cfg(feature = "pci")]
pub mod pci;

use alloc::collections::VecDeque;

#[cfg(feature = "pci")]
pub(crate) use pci_types::InterruptLine;

use crate::{Arch, ArchTrait};
#[cfg(not(feature = "pci"))]
pub(crate) type InterruptLine = u8;

pub(crate) type InterruptHandlerQueue = VecDeque<fn()>;

/// A common error module for drivers.
/// [DriverError](error::DriverError) values will be
/// passed on to higher layers.
#[deprecated]
pub mod error {}

/// A trait to determine general driver information
#[allow(dead_code)]
pub(crate) trait Driver {
	/// Returns the interrupt number of the device
	fn get_interrupt_number(&self) -> InterruptLine;

	/// Returns the device driver name
	fn get_name(&self) -> &'static str;
}

pub(crate) fn init() {
	// Initialize PCI Drivers
	#[cfg(feature = "pci")]
	crate::drivers::pci::init();
	#[cfg(target_arch = "riscv64")]
	crate::arch::riscv64::kernel::init_drivers();

	Arch::install_handlers();
}
