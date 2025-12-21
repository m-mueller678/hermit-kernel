use crate::ArchTrait;
use crate::scheduler::CoreId;

pub mod kernel;
pub mod mm;

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
}
