use alloc::alloc::AllocError;
use core::mem::MaybeUninit;

use crate::PageSize;

pub fn init() {
	todo!()
}
pub fn allocate_contiguous<S: PageSize>(count: usize) -> Result<usize, AllocError> {
	todo!()
}
/// Allocates all pages or fails
/// # Safety
/// start must be multiple of page size
pub fn allocate_at<S: PageSize>(start: usize, count: usize) -> Result<(), AllocError> {
	todo!()
}
pub fn allocate_multiple<S: PageSize>(
	dst: &mut [MaybeUninit<usize>],
) -> Result<&mut [usize], AllocError> {
	todo!()
}
pub fn allocate<S: PageSize>() -> Result<usize, AllocError> {
	todo!()
}
pub unsafe fn deallocate_multiple<S: PageSize>(frames: &[usize]) {
	for &x in frames {
		unsafe { deallocate::<S>(x) }
	}
}
pub unsafe fn deallocate<S: PageSize>(frame: usize) {
	todo!()
}
