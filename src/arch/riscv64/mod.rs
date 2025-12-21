pub mod kernel;
pub mod mm;

#[allow(dead_code)]
#[inline(always)]
pub(crate) fn memory_barrier() {
	riscv::asm::sfence_vma_all();
}

pub struct Arch;

impl ArchTrait for Arch {
	fn set_oneshot_timer(wakeup_time: Option<u64>) {
		kernel::processor::set_oneshot_timer(wakeup_time);
	}

	fn wakeup_core(core_id_to_wakeup: CoreId) {
		kernel::processor::wakeup_core(core_id_to_wakeup);
	}

	fn application_processor_init() {
		kernel::application_processor_init();
	}
}
