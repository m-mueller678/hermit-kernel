//! Memory management.
//!
//! This is an overview of Hermit's memory layout:
//!
//! - `DeviceAlloc.device_offset` is 0 if `!cfg!(careful)`
//! - User space virtual memory is only used if `!cfg!(feature = "common-os")`
//! - On x86-64, PCI BARs, I/O APICs, and local APICs may be in `0xc0000000..0xffffffff`, which could be inside of `MEM`.
//!
//! ```text
//!                               Virtual address
//!                                    space
//!
//!                                 ...┌───┬──► 00000000
//!           Physical address   ...   │   │
//!                space      ...      │   │ Identity map
//!                        ...         │   │
//!    00000000 ◄──┬───┐...         ...├───┼──► mem_size
//!                │   │   ...   ...   │   │
//!     FrameAlloc │MEM│      ...      │   │ Unused
//!                │   │   ...   ...   │   │
//!    mem_size ◄──┼───┤...         ...├───┼──► DeviceAlloc.phys_offset
//!                │   │   ...         │   │
//!                │   │      ...      │   │ DeviceAlloc
//!                │   │         ...   │   │
//!          Empty │   │            ...├───┼──► DeviceAlloc.phys_offset + mem_size
//!                │   │               │   │
//!                │   │               │   │
//!                │   │               │   │ Unused
//!     Unknown ◄──┼───┤               │   │
//!                │   │               │   │
//!            PCI │   │               ├───┼──► kernel_virt_start
//!                │   │               │   │
//!     Unknown ◄──┼───┤               │   │ PageAlloc
//!                │   │               │   │
//!                │   │               ├───┼──► kernel_virt_end
//!                │   │               │   │
//!          Empty │   │               │   │
//!                │   │               │   │ User space
//!                │   │               │   │
//!                │   │               │   │
//! ```

use core::mem::MaybeUninit;
use core::num::NonZeroUsize;
use core::ptr;

use hermit_sync::RawInterruptTicketMutex;
use talc::{ErrOnOom, Talc, Talck};

use crate::arch::{PageFlags, PageFlagsTrait};
use crate::logging::format_binary_si_bytes;
use crate::mm::page_size::PageSize;
use crate::{Arch, ArchTrait, HEAP_PAGE_SIZE, MIN_PAGE_SIZE, PagingTrait};

pub mod page_dump;
pub mod page_size;
pub mod physical_memory;
pub mod range_diff;
pub mod virtual_memory;

#[global_allocator]
pub(crate) static ALLOCATOR: Talck<RawInterruptTicketMutex, ErrOnOom> = Talc::new(ErrOnOom).lock();

pub(crate) fn init() {
	// init physical and virtual allocators
	{
		let mut physical_memory = Arch::physical_mem();
		let identity_map_info = unsafe { Arch::init_identity_mapping(&mut physical_memory) };

		while let Some(memory) = physical_memory.take_remaining() {
			unsafe {
				physical_memory::claim(memory);
			}
		}

		virtual_memory::init();

		unsafe { Arch::claim_virtual_memory(identity_map_info) }

		info!(
			"Claimed physical memory: {}",
			format_binary_si_bytes(physical_memory::total_claimed_memory())
		);
	}

	// init heap
	{
		let heap_pages = physical_memory::total_claimed_memory() / 4 / HEAP_PAGE_SIZE.usize();
		let heap_virtual =
			virtual_memory::allocate(HEAP_PAGE_SIZE, NonZeroUsize::new(heap_pages).unwrap())
				.unwrap();

		for i in 0..heap_pages {
			unsafe {
				Arch::map(
					HEAP_PAGE_SIZE,
					heap_virtual.get() + HEAP_PAGE_SIZE * i,
					physical_memory::allocate(HEAP_PAGE_SIZE).unwrap(),
					PageFlags::normal().writable(),
				);
			}
		}

		unsafe {
			ALLOCATOR
				.lock()
				.claim(talc::Span::new(
					ptr::with_exposed_provenance_mut(heap_virtual.get()),
					ptr::with_exposed_provenance_mut(
						heap_virtual.get() + HEAP_PAGE_SIZE * heap_pages,
					),
				))
				.unwrap();
		}
	}
}

pub unsafe fn map_contiguous(
	page_size: PageSize,
	virtual_address: NonZeroUsize,
	physical_address: usize,
	count: NonZeroUsize,
	flags: PageFlags,
) {
	todo!();
}

pub unsafe fn unmap_contiguous(
	page_size: PageSize,
	virtual_address: NonZeroUsize,
	count: NonZeroUsize,
) -> NonZeroUsize {
	todo!()
}
