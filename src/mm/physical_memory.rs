use alloc::alloc::AllocError;
use core::mem::MaybeUninit;
use core::ops::Range;

use free_list::{FreeList, PageLayout, PageRange};
use hermit_sync::InterruptTicketMutex;

use crate::mm::page_size::PageSize;

pub fn init() {
	todo!()
}
pub fn allocate_contiguous(_page_size: PageSize, _count: usize) -> Result<usize, AllocError> {
	todo!()
}
/// Allocates all pages or fails
/// # Safety
/// start must be multiple of page size
pub fn allocate_at(_page_size: PageSize, _start: usize, _count: usize) -> Result<(), AllocError> {
	todo!()
}
pub fn allocate_multiple(
	page_size: PageSize,
	dst: &mut [MaybeUninit<usize>],
) -> Result<(), AllocError> {
	for i in 0..dst.len() {
		match allocate(page_size) {
			Ok(x) => {
				dst[i].write(x);
			}
			Err(_) => unsafe {
				deallocate_multiple(page_size, dst[..i].assume_init_ref());
			},
		}
	}
	Ok(())
}
pub fn allocate(page_size: PageSize) -> Result<usize, AllocError> {
	FREE_LIST
		.lock()
		.allocate(PageLayout::from_size_align(page_size.usize(), page_size.usize()).unwrap())
		.map(|x| x.start())
		.map_err(|_| AllocError)
}
pub unsafe fn deallocate_multiple(page_size: PageSize, frames: &[usize]) {
	for &x in frames {
		unsafe { deallocate(page_size, x) }
	}
}
pub unsafe fn deallocate(page_size: PageSize, frame: usize) {
	unsafe {
		FREE_LIST
			.lock()
			.deallocate(PageRange::from_start_len(frame, page_size.usize()).unwrap())
			.unwrap();
	}
}

pub unsafe fn claim(range: Range<usize>) {
	unsafe {
		FREE_LIST
			.lock()
			.deallocate(PageRange::new(range.start, range.end).unwrap())
			.unwrap();
	}
}

static FREE_LIST: InterruptTicketMutex<FreeList<32>> = InterruptTicketMutex::new(FreeList::new());
