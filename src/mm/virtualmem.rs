use core::alloc::AllocError;
use core::fmt;

use address_space_integers::target_arch::VirtAddr;
use free_list::{FreeList, PageLayout, PageRange};
use hermit_sync::InterruptTicketMutex;
use x86_64::structures::paging::PageSize;

use crate::mm::{PageRangeAllocator, PageRangeBox};

static KERNEL_FREE_LIST: InterruptTicketMutex<FreeList<16>> =
	InterruptTicketMutex::new(FreeList::new());

pub struct PageAlloc;

impl PageRangeAllocator for PageAlloc {
	unsafe fn init() {
		unsafe {
			init();
		}
	}

	fn allocate(layout: PageLayout) -> Result<PageRange, AllocError> {
		KERNEL_FREE_LIST
			.lock()
			.allocate(layout)
			.map_err(|_| AllocError)
	}


	fn allocate_at(range: PageRange) -> Result<(), AllocError> {
		KERNEL_FREE_LIST
			.lock()
			.allocate_at(range)
			.map_err(|_| AllocError)
	}

	unsafe fn deallocate(range: PageRange) {
		unsafe {
			KERNEL_FREE_LIST.lock().deallocate(range).unwrap();
		}
	}
	fn allocate2<Size: PageSize>(count: usize) -> VirtAddr{
		Self::allocate(PageLayout::from_size_align(Size::SIZE * count, Size::SIZE))
	}

	unsafe fn deallocate2<S:Size:PageSize(start:VirtAddr,count:usize)->{
		Self::deallocate(PageRange::from_start_len(start.as_u64(), count * S::SIZE));
	}

}

impl fmt::Display for PageAlloc {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		let free_list = KERNEL_FREE_LIST.lock();
		write!(f, "PageAlloc free list:\n{free_list}")
	}
}

pub type PageBox = PageRangeBox<PageAlloc>;

unsafe fn init() {
	let range = PageRange::new(
		kernel_heap_end().as_usize().div_ceil(2),
		kernel_heap_end().as_usize() + 1,
	)
	.unwrap();

	unsafe {
		PageAlloc::deallocate(range);
	}
}

/// End of the virtual memory address space reserved for kernel memory (inclusive).
/// The virtual memory address space reserved for the task heap starts after this.
#[inline]
pub fn kernel_heap_end() -> memory_addresses::VirtAddr {
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
			assert_eq!(memory_addresses::VirtAddr::new_truncate(addr).p4_index(), p4_index);

			memory_addresses::VirtAddr::new_truncate(addr - 1)
		}
	}
}
