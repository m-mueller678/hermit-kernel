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

use core::num::NonZeroUsize;

use hermit_sync::RawInterruptTicketMutex;
use talc::{ErrOnOom, Talc, Talck};

use crate::mm::page_size::PageSize;
use crate::{Arch, ArchTrait, PageFlags, PagingTrait};

pub mod page_dump;
pub mod page_size;
pub mod physical_memory;
pub mod range_diff;
pub mod virtual_memory;

#[global_allocator]
pub(crate) static ALLOCATOR: Talck<RawInterruptTicketMutex, ErrOnOom> = Talc::new(ErrOnOom).lock();

pub(crate) fn init() {
	let mut physical_memory = Arch::physical_mem();
	let identity_map_info = unsafe { Arch::init_identity_mapping(&mut physical_memory) };

	while let Some(memory) = physical_memory.take_remaining() {
		unsafe {
			physical_memory::claim(memory);
		}
	}

	virtual_memory::init();

	unsafe { Arch::claim_virtual_memory(identity_map_info) }

	// info!("Total memory size: {} MiB", total_mem >> 20);
	// info!(
	// 	"Kernel region: {:p}..{:p}",
	// 	kernel_addr_range.start, kernel_addr_range.end
	// );

	// put some memory into ALLOCATOR and print information
	todo!()
	// info!("Heap is located at {heap_start_addr:p}..{heap_end_addr:p}");
}

/// Maps a given physical address and size. Allocated appropriate virtual memory and returns virtual address
#[cfg(feature = "pci")]
#[deprecated]
pub(crate) fn map(
	physical_address: usize,
	size_bytes: usize,
	writable: bool,
	no_execution: bool,
	no_cache: bool,
) -> usize {
	unimplemented!()
}

#[deprecated]
/// unmaps virtual address, without 'freeing' physical memory it is mapped to!
pub(crate) fn unmap(virtual_address: usize, size: usize) {
	unimplemented!()
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
