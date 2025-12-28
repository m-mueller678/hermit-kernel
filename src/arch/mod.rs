//! Architecture-specific architecture abstraction.

use crate::errno::Errno;
use crate::scheduler::CoreId;

pub trait ArchTrait: PagingTrait {
	fn set_oneshot_timer(wakeup_time: Option<u64>);
	fn wakeup_core(core_id_to_wakeup: CoreId);

	fn boot_processor_init();
	fn application_processor_init();

	// Todo rename these
	fn enable_and_wait();
	fn install_handlers();
	fn enable();
	fn disable();

	type SerialDevice: Default
		+ embedded_io::Read
		+ embedded_io::Write
		+ embedded_io::ReadReady
		+ embedded_io::ErrorType<Error = Errno>
		+ Send
		+ Sync;

	#[cfg(feature = "pci")]
	fn init_pci();
	#[cfg(feature = "pci")]
	type PciConfigRegion: ConfigRegionAccess;

	fn shutdown(code: i32) -> !;
	fn get_timestamp() -> u64;
	fn detect_timestamp_frequency() -> Option<(u64, &'static str)>;
	/// time at which `get_timestamp` was zero, expressed in microseconds since unix epoch
	fn timestamp_unix_offset() -> u64;

	fn get_entropy() -> Option<[u8; 32]>;
	fn args() -> Option<&'static str>;
	fn get_possible_cpus() -> u32;
	fn boot_next_processor();

	type DevicePageSize: PageSize;
	type IdentityPageSize: PageSize;
	type HeapPageSize: PageSize;
	type MinPageSize: PageSize;

	fn print_statistics();
}

pub trait PageFlagsTrait {
	fn normal() -> Self;
	fn device() -> Self;
	fn executable(self) -> Self;
	fn writable(self) -> Self;
}

pub unsafe trait PagingTrait {
	type Flags: PageFlagsTrait;
	unsafe fn init_paging();
	unsafe fn merge_page<S: PageSize>(address: usize);
	unsafe fn split_page<S: PageSize>(address: usize);
	/// # Safety
	/// physical_address must be a free physical frame of size S
	/// virtual_address must be an unmapped page os size S currently configured for size S
	unsafe fn map<S: PageSize>(virtual_address: usize, physical_address: usize, flags: Self::Flags);
	unsafe fn unmap<S: PageSize>(virtual_address: usize) -> usize;
}

pub trait PageSize: arch_impl::ArchPageSize {
	fn size() -> usize;
}

pub use arch_impl::Arch;
use pci_types::ConfigRegionAccess;
// pub type Arch = impl ArchTrait;

// #[define_opaque(Arch)]
// fn _constrain_arch() -> Arch {
// 	arch_impl::Arch
// }

macro_rules! forward_type {
	($T:ident) => {
		pub type $T = <Arch as ArchTrait>::$T;
	};
}

forward_type!(SerialDevice);
#[cfg(feature = "pci")]
forward_type!(PciConfigRegion);
forward_type!(DevicePageSize);
forward_type!(IdentityPageSize);
forward_type!(HeapPageSize);
forward_type!(MinPageSize);
pub type PageFlags = <Arch as PagingTrait>::Flags;

cfg_if::cfg_if! {
	if #[cfg(target_arch = "aarch64")] {
		pub(crate) mod aarch64;
		pub(crate) use self::aarch64::*;
		pub(crate) use self::aarch64::Arch;

		pub(crate) use self::aarch64::kernel::boot_processor_init;
		pub(crate) use self::aarch64::kernel::core_local;
		pub(crate) use self::aarch64::kernel::interrupts;
		#[cfg(feature = "pci")]
		pub(crate) use self::aarch64::kernel::pci;
		pub(crate) use self::aarch64::kernel::processor;
		pub(crate) use self::aarch64::kernel::serial::SerialDevice;
		pub(crate) use self::aarch64::kernel::scheduler;
		pub(crate) use self::aarch64::kernel::{
			get_processor_count,
		};
		pub use self::aarch64::mm::paging::{BasePageSize, PageSize};
	} else if #[cfg(target_arch = "x86_64")] {
		pub(crate) mod x86_64;
		use x86_64 as arch_impl;

		pub(crate) use self::x86_64::kernel::core_local;
		pub(crate) use self::x86_64::kernel::gdt::set_current_kernel_stack;
		pub(crate) use self::x86_64::kernel::processor;
		pub(crate) use self::x86_64::kernel::scheduler;
		pub(crate) use self::x86_64::kernel::switch;
		pub(crate) use self::x86_64::kernel::{
			get_processor_count,
		};
	} else if #[cfg(target_arch = "riscv64")] {
		pub(crate) mod riscv64;
		pub(crate) use self::riscv64::*;

		#[cfg(feature = "pci")]
		pub(crate) use self::riscv64::kernel::pci;
		pub(crate) use self::riscv64::kernel::processor::{self};
		pub(crate) use self::riscv64::kernel::serial::SerialDevice;
		pub(crate) use self::riscv64::kernel::{
			boot_processor_init,
			core_local,
			get_processor_count,
			interrupts,
			scheduler,
			switch,
		};
		pub use self::riscv64::mm::paging::{BasePageSize, PageSize};
	}
}
