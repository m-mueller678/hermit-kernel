use core::alloc::AllocError;
use core::marker::PhantomData;
use core::mem::ManuallyDrop;
use core::ops::Deref;

use address_space_integers::paging::PageSize;
use address_space_integers::target_arch::VirtAddr;
use address_space_integers::{Address, AddressSpace};
use free_list::{PageLayout, PageRange};

/// An allocator that allocates memory in page granularity.
pub trait PageRangeAllocator<A: AddressSpace, P: PageSize> {
	fn init();

	/// Attempts to allocate a range of memory in page granularity.
	fn allocate(size: usize) -> Result<Address<A>, AllocError>;

	/// Attempts to allocate the pages described by `range`.
	fn allocate_at(addr: VirtAddr, size: usize) -> Result<(), AllocError>;

	/// Deallocates the pages described by `range`.
	///
	/// # Safety
	///
	/// - `range` must described a range of pages _currently allocated_ via this allocator.
	unsafe fn deallocate(addr: Address<A>, size: Address<A>);
}

pub struct PageRangeBox<A: AddressSpace, P: PageSize> {
	addr: Address<A>,
	size: Address<A>,
	_p: PhantomData<P>,
}

impl<A: AddressSpace, P: PageSize> PageRangeBox<A, P> {
	pub fn new(size: Address<A>) -> Result<Self, AllocError> {
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
