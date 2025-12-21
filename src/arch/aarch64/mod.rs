use aarch64_cpu::asm::barrier::{ISH, dmb};

pub mod kernel;
pub mod mm;

/// Force strict CPU ordering, serializes load and store operations.
#[allow(dead_code)]
#[inline(always)]
pub(crate) fn memory_barrier() {
	dmb(ISH);
}

pub struct Arch;

impl ArchTrait for Arch {
	fn set_oneshot_timer(wakeup_time: Option<u64>) {
		kernel::apic::set_oneshot_timer(wakeup_time);
	}

	fn wakeup_core(core_id_to_wakeup: CoreId) {
		kernel::apic::wakeup_core(core_id_to_wakeup);
	}

	fn application_processor_init() {
		kernel::application_processor_init();
	}
}
