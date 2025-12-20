use core::alloc::AllocError;
use core::marker::PhantomData;
use core::mem::ManuallyDrop;
use core::ops::{Deref, Range};

use address_space_integers::paging::PageSize;
use address_space_integers::target_arch::{PhysAddr, VirtAddr};
use address_space_integers::{Address, AddressSpace};
use free_list::{PageLayout, PageRange};

/// An allocator that allocates memory in page granularity.
// TODO eliminate old interface and move generic into trait
pub trait PageRangeAllocator<A: AddressSpace, P: PageSize> {
	unsafe fn init();

	fn allocate(layout: PageLayout) -> Result<PageRange, AllocError>;

	/// Deallocates the pages described by `range`.
	///
	/// # Safety
	///
	/// - `range` must described a range of pages _currently allocated_ via this allocator.
	unsafe fn deallocate(range: PageRange);
	/// Attempts to allocate the pages described by `range`.
	fn allocate_at(range: Range<Address<A>>) -> Result<(), AllocError>;

	/// Attempts to allocate a range of memory in page granularity.
	fn allocate<Size: PageSize>(count: usize) -> Result<VirtAddr, AllocError>;

	unsafe fn deallocate2<S: address_space_integers::paging::PageSize>(range: Range<Address<A>>);
}

pub struct PageRangeBox<A: PageRangeAllocator>(PageRange, PhantomData<A>);

impl<A: PageRangeAllocator> PageRangeBox<A> {
	pub fn new(layout: PageLayout) -> Result<Self, AllocError> {
		let range = A::allocate(layout)?;
		Ok(Self(range, PhantomData))
	}

	pub unsafe fn from_raw(range: PageRange) -> Self {
		Self(range, PhantomData)
	}

	pub fn into_raw(b: Self) -> PageRange {
		let b = ManuallyDrop::new(b);
		**b
	}
}

impl<A: PageRangeAllocator> Drop for PageRangeBox<A> {
	fn drop(&mut self) {
		unsafe {
			A::deallocate(self.0);
		}
	}
}

impl<A: PageRangeAllocator> Deref for PageRangeBox<A> {
	type Target = PageRange;

	fn deref(&self) -> &Self::Target {
		&self.0
	}
}
