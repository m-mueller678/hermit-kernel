use core::sync::atomic::AtomicU64;
use core::sync::atomic::Ordering::*;
use core::{mem, ptr};

use x86_64::PhysAddr;
use x86_64::registers::control::{Cr2, Cr3};
pub use x86_64::structures::idt::InterruptStackFrame as ExceptionStackFrame;
use x86_64::structures::idt::PageFaultErrorCode;
pub use x86_64::structures::paging::PageTableFlags as PageTableEntryFlags;
use x86_64::structures::paging::PhysFrame;
use x86_64::structures::paging::page_table::PageTableEntry;

use crate::arch::x86_64::kernel::processor;
use crate::arch::x86_64::{Size2MiB, Size4KiB};
use crate::mm::physical_memory;
use crate::{PageFlagsTrait, PageSize, PagingTrait};

#[non_exhaustive]
pub struct PagingInitToken {}

fn entry_from_raw(x: u64) -> PageTableEntry {
	unsafe { mem::transmute(x) }
}
fn entry_to_raw(x: PageTableEntry) -> u64 {
	unsafe { mem::transmute(x) }
}
fn node_from_address(addr: usize) -> &'static [AtomicU64; 512] {
	unsafe { ptr::with_exposed_provenance::<[AtomicU64; 512]>(addr).as_ref_unchecked() }
}

fn to_child<'a>(node: &[AtomicU64; 512], index: usize) -> &[AtomicU64; 512] {
	let entry = entry_from_raw(node[index].load(Relaxed));
	let child_addr = entry.frame().unwrap().start_address().as_u64() as usize;
	node_from_address(child_addr)
}

fn frame_from_address<S: PageSize>(addr: usize) -> PhysFrame {
	debug_assert!(addr.is_multiple_of(S::size()));
	unsafe { PhysFrame::from_start_address_unchecked(PhysAddr::new_unsafe(addr as u64)) }
}

fn make_entry<S: PageSize>(address: usize, flags: PageTableEntryFlags) -> u64 {
	let frame = frame_from_address::<S>(address);
	let mut entry = PageTableEntry::new();
	entry.set_frame(frame, flags);
	entry_to_raw(entry)
}
fn walk_to_containing_table<S: PageSize>(address: usize) -> (&'static [AtomicU64; 512], usize) {
	let mut indices = address >> 12;
	let mut entry_size = 1 << (12 + 3 * 9);
	let mut node = table_root_node();
	loop {
		let index = indices & 511;
		if entry_size == S::size() {
			return (node, index);
		}
		node = to_child(node, index);
		entry_size >>= 9;
		indices >>= 9;
	}
}
pub fn table_root_node() -> &'static [AtomicU64; 512] {
	let addr = Cr3::read().0.start_address().as_u64().try_into().unwrap();
	node_from_address(addr)
}

unsafe impl PagingTrait for crate::x86_64::Arch {
	unsafe fn init_paging() {
		todo!();
	}
	type Flags = PageFlags;
	unsafe fn merge_page<S: PageSize>(address: usize) {
		assert!(S::size() > Size4KiB::size());
		let (table, index) = walk_to_containing_table::<S>(address);
		let child_frame = entry_from_raw(table[index].load(Relaxed));
		debug_assert_eq!(child_frame.flags(), PageTableEntryFlags::PRESENT);
		let child_frame = child_frame.frame().unwrap();
		let child_frame = child_frame.start_address().as_u64() as usize;
		if cfg!(debug_assertions) {
			let child_size = S::size() >> 9;
			let child_is_huge = child_size > Size4KiB::size();
			let expected_flags = if child_is_huge {
				PageTableEntryFlags::HUGE_PAGE
			} else {
				PageTableEntryFlags::empty()
			};
			for x in node_from_address(child_frame) {
				assert_eq!(entry_from_raw(x.load(Relaxed)).flags(), expected_flags);
			}
		}
		unsafe {
			physical_memory::deallocate::<S>(child_frame);
		}
	}

	unsafe fn split_page<S: PageSize>(address: usize) {
		assert!(S::size() > Size4KiB::size());
		let (table, index) = walk_to_containing_table::<S>(address);
		let entry = entry_from_raw(table[index].load(Relaxed));
		assert_eq!(entry.flags(), PageTableEntryFlags::HUGE_PAGE);
		let child_frame = physical_memory::allocate::<Size4KiB>().unwrap();
		{
			let child_frame = node_from_address(child_frame);
			let mut child_entry = PageTableEntry::new();
			child_entry.set_flags(if S::size() > Size2MiB::size() {
				PageTableEntryFlags::HUGE_PAGE
			} else {
				PageTableEntryFlags::empty()
			});
			let child_entry = entry_to_raw(child_entry);
			for x in child_frame {
				x.store(child_entry, Relaxed);
			}
		}
		let mut entry = PageTableEntry::new();
		entry.set_frame(
			frame_from_address::<Size4KiB>(child_frame),
			PageTableEntryFlags::PRESENT,
		);
		table[index].store(entry_to_raw(entry), Relaxed);
	}

	/// # Safety
	/// physical_address must be a free physical frame of size S
	/// virtual_address must be an unmapped page os size S currently configured for size S
	unsafe fn map<S: PageSize>(virtual_address: usize, physical_address: usize, flags: PageFlags) {
		let entry: u64 = {
			let flags = if S::size() == 1 << 12 {
				flags.0
			} else {
				flags.0 | PageTableEntryFlags::HUGE_PAGE
			};
			let mut entry = PageTableEntry::new();
			entry.set_frame(frame_from_address::<S>(physical_address), flags);
			entry_to_raw(entry)
		};

		let (table, index) = walk_to_containing_table::<S>(virtual_address);
		table[index].store(entry, Relaxed);
	}

	unsafe fn unmap<S: PageSize>(virtual_address: usize) -> usize {
		let (table, index) = walk_to_containing_table::<S>(virtual_address);
		let is_huge = S::size() > Size4KiB::size();
		let empty_flags = if is_huge {
			PageTableEntryFlags::HUGE_PAGE
		} else {
			PageTableEntryFlags::empty()
		};
		let empty_entry = make_entry::<S>(0, empty_flags);
		let old_entry = table[index].swap(empty_entry, Relaxed);
		let old_entry = entry_from_raw(old_entry);
		debug_assert!(old_entry.flags().contains(PageTableEntryFlags::PRESENT));
		assert_eq!(
			is_huge,
			old_entry.flags().contains(PageTableEntryFlags::HUGE_PAGE)
		);
		let phys_addr = old_entry.addr().as_u64() as usize;
		debug_assert!(phys_addr.is_multiple_of(S::size()));
		// the paging module has no problem unmapping this page, but the kernel does not touch the identity mappings after setting them up.
		// This indicates a likely bug
		debug_assert!(phys_addr != virtual_address);
		phys_addr
	}
}

pub struct PageFlags(PageTableEntryFlags);

impl PageFlagsTrait for PageFlags {
	fn normal() -> Self {
		Self(
			PageTableEntryFlags::PRESENT
				| PageTableEntryFlags::ACCESSED
				| PageTableEntryFlags::DIRTY
				| PageTableEntryFlags::NO_EXECUTE
				| PageTableEntryFlags::PRESENT,
		)
	}

	fn device() -> Self {
		Self(Self::normal().0 | PageTableEntryFlags::NO_CACHE)
	}

	fn executable(self) -> Self {
		Self(self.0 & !PageTableEntryFlags::NO_EXECUTE)
	}

	fn writable(self) -> Self {
		Self(self.0 | PageTableEntryFlags::WRITABLE)
	}
}

pub(crate) extern "x86-interrupt" fn page_fault_handler(
	stack_frame: ExceptionStackFrame,
	error_code: PageFaultErrorCode,
) {
	long_panic!(
		("Page fault (#PF)!"),
		("page_fault_linear_address = {:p}", Cr2::read().unwrap()),
		("error_code = {error_code:?}"),
		("fs = {:#X}", processor::readfs()),
		("gs = {:#X}", processor::readgs()),
		("stack_frame = {stack_frame:#?}"),
	);
}
