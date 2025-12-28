pub mod kernel;
mod paging;

use crate::scheduler::CoreId;
use crate::{ArchTrait, PageSize};

/// Force strict CPU ordering, serializes load and store operations.
#[allow(dead_code)]
#[inline(always)]
pub(crate) fn memory_barrier() {
	use core::arch::asm;
	unsafe {
		asm!("mfence", options(nostack, nomem, preserves_flags));
	}
}

pub struct Arch;

impl ArchTrait for Arch {
	fn set_oneshot_timer(wakeup_time: Option<u64>) {
		kernel::apic::set_oneshot_timer(wakeup_time);
	}

	fn wakeup_core(core_id_to_wakeup: CoreId) {
		kernel::apic::wakeup_core(core_id_to_wakeup);
	}

	type SerialDevice = kernel::serial::SerialDevice;
	fn application_processor_init() {
		kernel::application_processor_init();
	}

	fn boot_processor_init() {
		kernel::boot_processor_init();
	}

	fn enable_and_wait() {
		kernel::interrupts::enable_and_wait();
	}

	fn install_handlers() {
		kernel::interrupts::install_handlers();
	}

	#[inline]
	fn enable() {
		kernel::interrupts::enable();
	}

	#[inline]
	fn disable() {
		kernel::interrupts::disable();
	}

	#[cfg(feature = "pci")]
	fn init_pci() {
		kernel::pci::init();
	}

	#[cfg(feature = "pci")]
	type PciConfigRegion = kernel::pci::PciConfigRegion;

	fn shutdown(code: i32) -> ! {
		kernel::processor::shutdown(code)
	}

	fn get_timestamp() -> u64 {
		kernel::processor::get_timestamp()
	}

	fn detect_timestamp_frequency() -> Option<(u64, &'static str)> {
		kernel::processor::detect_cpu_frequency()
	}

	fn get_entropy() -> Option<[u8; 32]> {
		kernel::processor::seed_entropy()
	}

	fn args() -> Option<&'static str> {
		kernel::args()
	}

	fn get_possible_cpus() -> u32 {
		kernel::apic::local_apic_id_count()
	}

	fn boot_next_processor() {
		kernel::boot_next_processor();
	}

	type DevicePageSize = Size4KiB;
	type HeapPageSize = Size4KiB;
	type IdentityPageSize = Size2MiB;
	type MinPageSize = Size4KiB;

	fn timestamp_unix_offset() -> u64 {
		kernel::systemtime::timestamp_unix_offset()
	}

	fn print_statistics() {
		kernel::print_statistics();
	}
}

pub trait ArchPageSize {}

macro_rules! define_page_size {
	($Name:ident,$size:expr) => {
		pub struct $Name;
		impl PageSize for $Name {
			fn size() -> usize {
				$size
			}
		}
		impl ArchPageSize for $Name {}
	};
}

define_page_size!(Size4KiB, 1 << 12);
define_page_size!(Size2MiB, 1 << 21);
