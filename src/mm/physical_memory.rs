use alloc::alloc::AllocError;
use core::mem::MaybeUninit;
use core::ops::Range;

use crate::mm::page_size::PageSize;

pub fn init() {
	todo!()
}
pub fn allocate_contiguous(page_size: PageSize, count: usize) -> Result<usize, AllocError> {
	todo!()
}
/// Allocates all pages or fails
/// # Safety
/// start must be multiple of page size
pub fn allocate_at(page_size: PageSize, start: usize, count: usize) -> Result<(), AllocError> {
	todo!()
}
pub fn allocate_multiple(
	page_size: PageSize,
	dst: &mut [MaybeUninit<usize>],
) -> Result<&mut [usize], AllocError> {
	todo!()
}
pub fn allocate(page_size: PageSize) -> Result<usize, AllocError> {
	todo!()
}
pub unsafe fn deallocate_multiple(page_size: PageSize, frames: &[usize]) {
	for &x in frames {
		unsafe { deallocate(page_size, x) }
	}
}
pub unsafe fn deallocate(page_size: PageSize, frame: usize) {
	todo!()
}

pub unsafe fn claim(range: Range<usize>) {
	todo!()
}
