use free_list::{PageLayout, PageRange};

use crate::mm::{FrameAlloc, PageAlloc, PageRangeAllocator};

/// Allocate physical memory.
#[hermit_macro::system]
#[unsafe(no_mangle)]
pub extern "C" fn sys_allocate_physical(size: usize, align: usize) -> usize {
	match FrameAlloc::allocate(PageLayout::from_size_align(size, align).unwrap()) {
		Ok(x) => x.start(),
		Err(_) => usize::MAX,
	}
}

/// Deallocate physical memory previously allocated with [sys_allocate_physical].
#[hermit_macro::system]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sys_deallocate_physical(addr: usize, size: usize) {
	unsafe { FrameAlloc::deallocate(PageRange::from_start_len(addr, size).unwrap()) };
}

/// Allocate virtual memory.
#[hermit_macro::system]
#[unsafe(no_mangle)]
pub extern "C" fn sys_allocate_virtual(size: usize, align: usize) -> usize {
	match PageAlloc::allocate(PageLayout::from_size_align(size, align).unwrap()) {
		Ok(x) => x.start(),
		Err(_) => usize::MAX,
	}
}

/// Deallocate virtual memory previously allocated with [sys_allocate_virtual].
#[hermit_macro::system]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sys_deallocate_virtual(addr: usize, size: usize) {
	unsafe { PageAlloc::deallocate(PageRange::from_start_len(addr, size).unwrap()) };
}

#[hermit_macro::system]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sys_global_tlb_flush() {
	cfg_if::cfg_if!(
		if #[cfg(target_arch = "x86_64")]{
			// #[cfg(feature="smp")]
			// crate::arch::x86_64::kernel::apic::ipi_tlb_flush();
			x86_64::structures::paging::mapper::MapperFlushAll::new().flush_all();
		}else{
			unimplemented!();
		}
	);
}
