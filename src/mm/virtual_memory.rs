use alloc::alloc::AllocError;
use core::num::NonZeroUsize;

use crate::PageSize;

pub fn init() {
	todo!()
}

/// Attempts to allocate a range of memory in page granularity.
pub fn allocate<S: PageSize>(count: NonZeroUsize) -> Result<NonZeroUsize, AllocError> {
	todo!()
}

// /// Attempts to allocate pages.
// /// # Safety
// /// start must be a non-zero multiple of the page size
// pub fn allocate_at<S: PageSize>(start: NonZeroUsize, count: NonZeroUsize)
// -> Result<(), AllocError>{todo!()}

/// Deallocates.
///
/// # Safety
/// - All pages in the range must be currently allocated
/// start must be aligned to the page size
pub unsafe fn deallocate<S: PageSize>(start: NonZeroUsize, count: NonZeroUsize) {
	todo!()
}
