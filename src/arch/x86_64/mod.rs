pub mod kernel;
mod paging;

use crate::mm::range_diff::RangeDiff;
use crate::scheduler::CoreId;
use crate::{ArchTrait, PageSize, env};

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

	fn physical_mem() -> RangeDiff {
		let fdt = env::fdt().unwrap();
		let fdt_start = env::boot_info().hardware_info.device_tree.unwrap().get() as usize;
		let fdt_end = fdt_start + fdt.total_size();
		let fdt_region = fdt_start..fdt_end;
		let fdt_reserved_regions = fdt.memory_reservations().map(|r| {
			let start = r.address() as usize;
			let end = start + r.size() as usize;
			start..end
		});
		const FREE_LIST_INLINE_SIZE: usize = 64;

		let kernel_region = {
			let kernel_range = env::boot_info().load_info.kernel_image_addr_range.clone();
			let start = if env::is_uefi() {
				kernel_range.start as usize
			} else {
				// FIXME: memory before the kernel causes trouble on non-uefi systems.
				// It is unclear, which exact regions cause problems
				0
			};
			start..(kernel_range.end as usize)
		};
		let reserved_regions = core::iter::once(kernel_region)
			.chain(core::iter::once(fdt_region))
			.chain(fdt_reserved_regions);
		let memories = fdt.find_all_nodes("/memory").map(|m| {
			let region = m.reg().unwrap().next().unwrap();
			let start = region.starting_address.addr();
			start..start + region.size.unwrap()
		});
		RangeDiff::new(memories, reserved_regions)
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
