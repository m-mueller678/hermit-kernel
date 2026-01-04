use alloc::alloc::AllocError;
use core::num::NonZeroUsize;

use free_list::{FreeList, PageLayout, PageRange};
use hermit_sync::InterruptTicketMutex;

use crate::arch::NUM_PAGE_SIZES;
use crate::mm::page_size::PageSize;

pub fn init() {}

/// Attempts to allocate a range of memory in page granularity.
pub fn allocate(page_size: PageSize, count: NonZeroUsize) -> Result<NonZeroUsize, AllocError> {
	match FREE_LISTS[page_size.size_index()].lock().allocate(
		dbg!(PageLayout::from_size_align(
			dbg!(page_size) * dbg!(count.get()),
			page_size.usize()
		))
		.unwrap(),
	) {
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
			.unwrap();
	}
}

pub unsafe fn claim_pages(page_size: PageSize, start: NonZeroUsize, count: NonZeroUsize) {
	// TODO: free-list has a bug with handling usive overflows, stay away from the end of the address space as a work around.
	// 1<<42 is an arbitrary large power of two.
	let max_end = 0usize.wrapping_sub(1 << 42);
	if max_end <= start.get() {
		return;
	}
	let max_count = NonZeroUsize::new((max_end - start.get()) / page_size).unwrap();
	unsafe { deallocate(page_size, start, count.min(max_count)) }
}

static FREE_LISTS: [InterruptTicketMutex<FreeList<32>>; NUM_PAGE_SIZES] =
	[const { InterruptTicketMutex::new(FreeList::new()) }; NUM_PAGE_SIZES];
