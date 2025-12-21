use core::alloc::AllocError;
use core::fmt;
use core::num::NonZeroUsize;
use core::sync::atomic::AtomicBool;
use core::sync::atomic::Ordering::Relaxed;

use free_list::{FreeList, PageLayout, PageRange};
use hermit_sync::InterruptTicketMutex;
use memory_addresses::VirtAddr;

use crate::mm::{PageRangeAllocator, PageRangeBox, PageSize};

pub trait VirtualAllocator {
	fn init();

	/// Attempts to allocate a range of memory in page granularity.
	fn allocate<S: PageSize>(count: NonZeroUsize) -> Result<NonZeroUsize, AllocError>;

	// /// Attempts to allocate pages.
	// /// # Safety
	// /// start must be a non-zero multiple of the page size
	// fn allocate_at<S: PageSize>(start: NonZeroUsize, count: NonZeroUsize)
	// -> Result<(), AllocError>;

	/// Deallocates.
	///
	/// # Safety
	/// - All pages in the range must be currently allocated
	/// start must be aligned to the page size
	unsafe fn deallocate<S: PageSize>(start: usize, count: NonZeroUsize);
}

static KERNEL_FREE_LIST: InterruptTicketMutex<FreeList<16>> =
	InterruptTicketMutex::new(FreeList::new());

pub struct PageAlloc;

impl VirtualAllocator for PageAlloc {
	fn init() {
		static ONCE: AtomicBool = AtomicBool::new(false);
		assert!(!ONCE.swap(true, Relaxed));
		{
			unsafe {
				KERNEL_FREE_LIST.lock().deallocate(
					PageRange::new(
						kernel_heap_end().as_usize().div_ceil(2),
						kernel_heap_end().as_usize() + 1,
					)
					.unwrap(),
				);
			}
		};
	}

	fn allocate<S: PageSize>(count: NonZeroUsize) -> Result<NonZeroUsize, AllocError> {
		let size = S::SIZE.checked_mul(count).ok_or(AllocError)?;
		KERNEL_FREE_LIST
			.lock()
			.allocate(PageLayout::from_size_align(size, S::SIZE))
			.map_err(|_| AllocError)
	}

	unsafe fn deallocate<S: PageSize>(start: usize, count: NonZeroUsize) {
		unsafe {
			KERNEL_FREE_LIST
				.lock()
				.deallocate(PageRange::new(start, start + count.get() * S::SIZE))
				.unwrap();
		}
	}
}

impl fmt::Display for PageAlloc {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		let free_list = KERNEL_FREE_LIST.lock();
		write!(f, "PageAlloc free list:\n{free_list}")
	}
}

/// End of the virtual memory address space reserved for kernel memory (inclusive).
/// The virtual memory address space reserved for the task heap starts after this.
#[inline]
fn kernel_heap_end() -> VirtAddr {
	cfg_if::cfg_if! {
		if #[cfg(target_arch = "aarch64")] {
			// maximum address, which can be supported by TTBR0
			VirtAddr::new(0xFFFF_FFFF_FFFF)
		} else if #[cfg(target_arch = "riscv64")] {
			// 256 GiB
			VirtAddr::new(0x0040_0000_0000 - 1)
		} else if #[cfg(target_arch = "x86_64")] {
			use x86_64::structures::paging::PageTableIndex;

			let p4_index = PageTableIndex::new(256);

			let addr = u64::from(p4_index) << 39;
			assert_eq!(VirtAddr::new_truncate(addr).p4_index(), p4_index);

			VirtAddr::new_truncate(addr - 1)
		}
	}
}
