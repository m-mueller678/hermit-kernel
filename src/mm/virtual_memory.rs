use alloc::alloc::AllocError;
use core::num::NonZeroUsize;

use free_list::{FreeList, PageLayout, PageRange};
use hermit_sync::InterruptTicketMutex;

use crate::arch::NUM_PAGE_SIZES;
use crate::mm::page_size::PageSize;

pub fn init() {}

/// Attempts to allocate a range of memory in page granularity.
pub fn allocate(page_size: PageSize, count: NonZeroUsize) -> Result<NonZeroUsize, AllocError> {
	match FREE_LISTS[page_size.size_index()]
		.lock()
		.allocate(PageLayout::from_size_align(page_size * count.get(), page_size.usize()).unwrap())
	{
		Ok(x) => Ok(NonZeroUsize::new(x.start()).unwrap()),
		Err(_) => Err(AllocError),
	}
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
///
/// start must be aligned to the page size
pub unsafe fn deallocate(page_size: PageSize, start: NonZeroUsize, count: NonZeroUsize) {
	unsafe {
		FREE_LISTS[page_size.size_index()]
			.lock()
			.deallocate(PageRange::from_start_len(start.get(), page_size * count.get()).unwrap())
			.unwrap()
	}
}

pub unsafe fn claim_pages(page_size: PageSize, start: NonZeroUsize, count: NonZeroUsize) {
	unsafe { deallocate(page_size, start, count) }
}

static FREE_LISTS: [InterruptTicketMutex<FreeList<32>>; NUM_PAGE_SIZES] =
	[const { InterruptTicketMutex::new(FreeList::new()) }; NUM_PAGE_SIZES];
