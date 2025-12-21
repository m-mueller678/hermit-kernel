//! Architecture-specific architecture abstraction.

cfg_if::cfg_if! {
	if #[cfg(target_arch = "aarch64")] {
		pub(crate) mod aarch64;
		use arch64 as target;
	} else if #[cfg(target_arch = "x86_64")] {
		pub(crate) mod x86_64;
		use x86_64 as target;
	} else if #[cfg(target_arch = "riscv64")] {
		pub(crate) mod riscv64;
		use riscv64 as target;
	}
}

#[cfg(feature = "pci")]
pub(crate) use self::target::pci;
pub(crate) use self::target::{
	BasePageSize, PageSize, SerialDevice, application_processor_init, boot_processor_init,
	core_local, get_processor_count, interrupts, processor, scheduler, set_oneshot_timer,
	wakeup_core,
};
