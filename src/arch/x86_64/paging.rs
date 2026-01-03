use core::num::NonZeroUsize;
use core::ops::Range;
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
use crate::arch::x86_64::{SIZE_2MIB, SIZE_4KIB};
use crate::mm::page_size::PageSize;
use crate::mm::range_diff::RangeDiff;
use crate::mm::{physical_memory, virtual_memory};
use crate::x86_64::SIZE_1GIB;
use crate::{PageFlagsTrait, PageTableEntryDebug, PagingTrait};

fn entry_from_raw(x: u64) -> PageTableEntry {
	unsafe { mem::transmute(x) }
}
fn entry_to_raw(x: PageTableEntry) -> u64 {
	unsafe { mem::transmute(x) }
}
fn node_from_address(addr: usize) -> &'static [AtomicU64; 512] {
	unsafe { ptr::with_exposed_provenance::<[AtomicU64; 512]>(addr).as_ref_unchecked() }
}

fn to_child(node: &[AtomicU64; 512], index: usize) -> &[AtomicU64; 512] {
	let entry = entry_from_raw(node[index].load(Relaxed));
	let child_addr = entry.frame().unwrap().start_address().as_u64() as usize;
	node_from_address(child_addr)
}

fn frame_from_address(addr: usize) -> PhysFrame {
	unsafe { PhysFrame::from_start_address_unchecked(PhysAddr::new_unsafe(addr as u64)) }
}

fn make_entry(address: usize, flags: PageTableEntryFlags) -> u64 {
	let mut entry = PageTableEntry::new();
	entry.set_addr(PhysAddr::new(address as u64), flags);
	entry_to_raw(entry)
}
fn walk_to_containing_table(
	page_size: PageSize,
	address: usize,
) -> (&'static [AtomicU64; 512], usize) {
	let mut indices = address >> 12;
	let mut entry_size = 1 << (12 + 3 * 9);
	let mut node = table_root_node();
	loop {
		let index = indices & 511;
		if entry_size == page_size.usize() {
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
	unsafe fn init_identity_mapping(physical_mem: &mut RangeDiff) -> Self::IdentityMappingInfo {
		let l4 = table_root_node();
		let l3 = to_child(l4, 0);
		assert!(super::kernel::processor::supports_1gib_pages());
		let memory_end = physical_mem.memory_end();
		let num_identity_map = memory_end.div_ceil(1 << 30);
		dbg!(num_identity_map);
		assert!(
			num_identity_map <= 512,
			"mapping of more than 512GiB of memory is not yet implemented"
		);
		for (i, x) in l3.iter().enumerate() {
			x.store(
				if i < num_identity_map {
					make_entry(
						i << 30,
						PageTableEntryFlags::PRESENT
							| PageTableEntryFlags::HUGE_PAGE
							| PageTableEntryFlags::WRITABLE
							| PageTableEntryFlags::DIRTY
							| PageTableEntryFlags::ACCESSED,
					)
				} else {
					0
				},
				Relaxed,
			);
		}

		// hermit loader sets up recursive page tables (the last entry points to the table root).
		// We have no need for those.
		table_root_node()[511].store(0, Relaxed);

		x86_64::instructions::tlb::flush_all();

		// crate::mm::page_dump::dump_page_table_hierarchical();
		crate::mm::page_dump::dump_page_table_leaves();
		crate::mm::page_dump::dump_page_table_hierarchical();

		NumIdentityPage(num_identity_map)
	}
	unsafe fn claim_virtual_memory(identity_map_info: Self::IdentityMappingInfo) {
		let root = table_root_node();
		for entry in &root[1..] {
			let frame = physical_memory::allocate(SIZE_4KIB).unwrap();
			unsafe { ptr::with_exposed_provenance_mut::<[u64; 512]>(frame).as_mut_unchecked() }
				.fill(0);
			entry.store(
				make_entry(
					frame,
					PageTableEntryFlags::PRESENT
						| PageTableEntryFlags::WRITABLE
						| PageTableEntryFlags::ACCESSED,
				),
				Relaxed,
			);
		}
		fn claim_range_plus_end(range: Range<usize>) {
			unsafe {
				virtual_memory::claim_pages(
					SIZE_1GIB,
					NonZeroUsize::new(range.start).unwrap(),
					(range.end - range.start) / SIZE_1GIB.usize(),
				);
				virtual_memory::claim_pages(SIZE_2MIB, NonZeroUsize::new(range.end).unwrap(), 511);
				virtual_memory::claim_pages(
					SIZE_4KIB,
					NonZeroUsize::new(range.end + SIZE_2MIB * 511).unwrap(),
					511,
				);
			}
		}
		claim_range_plus_end(SIZE_1GIB * identity_map_info.0..1 << 47);
		claim_range_plus_end(usize::MAX << 47..0usize.wrapping_sub(SIZE_1GIB.usize()));
	}

	type IdentityMappingInfo = NumIdentityPage;
	type Flags = PageFlags;
	unsafe fn merge_page(larger_page: PageSize, address: usize) {
		assert!(larger_page > SIZE_4KIB);
		let (table, index) = walk_to_containing_table(larger_page, address);
		let child_frame = entry_from_raw(table[index].load(Relaxed));
		debug_assert_eq!(child_frame.flags(), PageTableEntryFlags::PRESENT);
		let child_frame = child_frame.frame().unwrap();
		let child_frame = child_frame.start_address().as_u64() as usize;
		if cfg!(debug_assertions) {
			let child_size = larger_page.usize() >> 9;
			let child_is_huge = child_size > SIZE_4KIB.usize();
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
			physical_memory::deallocate(SIZE_4KIB, child_frame);
		}
	}

	unsafe fn split_page(larger_page: PageSize, address: usize) {
		assert!(larger_page.usize() > SIZE_4KIB.usize());
		let (table, index) = walk_to_containing_table(larger_page, address);
		let entry = entry_from_raw(table[index].load(Relaxed));
		assert_eq!(entry.flags(), PageTableEntryFlags::HUGE_PAGE);
		let child_frame = physical_memory::allocate(SIZE_4KIB).unwrap();
		{
			let child_frame = node_from_address(child_frame);
			let mut child_entry = PageTableEntry::new();
			child_entry.set_flags(if larger_page > SIZE_2MIB {
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
			frame_from_address(child_frame),
			PageTableEntryFlags::PRESENT,
		);
		table[index].store(entry_to_raw(entry), Relaxed);
	}

	/// # Safety
	/// physical_address must be a free physical frame of size S
	/// virtual_address must be an unmapped page os size S currently configured for size S
	unsafe fn map(
		page_size: PageSize,
		virtual_address: usize,
		physical_address: usize,
		flags: PageFlags,
	) {
		let entry: u64 = {
			let flags = if page_size == SIZE_4KIB {
				flags.0
			} else {
				flags.0 | PageTableEntryFlags::HUGE_PAGE
			};
			let mut entry = PageTableEntry::new();
			entry.set_frame(frame_from_address(physical_address), flags);
			entry_to_raw(entry)
		};

		let (table, index) = walk_to_containing_table(page_size, virtual_address);
		table[index].store(entry, Relaxed);
	}

	unsafe fn unmap(page_size: PageSize, virtual_address: usize) -> usize {
		let (table, index) = walk_to_containing_table(page_size, virtual_address);
		let is_huge = page_size > SIZE_4KIB;
		let empty_flags = if is_huge {
			PageTableEntryFlags::HUGE_PAGE
		} else {
			PageTableEntryFlags::empty()
		};
		let empty_entry = make_entry(0, empty_flags);
		let old_entry = table[index].swap(empty_entry, Relaxed);
		let old_entry = entry_from_raw(old_entry);
		debug_assert!(old_entry.flags().contains(PageTableEntryFlags::PRESENT));
		assert_eq!(
			is_huge,
			old_entry.flags().contains(PageTableEntryFlags::HUGE_PAGE)
		);
		let phys_addr = old_entry.addr().as_u64() as usize;
		debug_assert!(phys_addr.is_multiple_of(page_size.usize()));
		// the paging module has no problem unmapping this page, but the kernel does not touch the identity mappings after setting them up.
		// This indicates a likely bug
		debug_assert!(phys_addr != virtual_address);
		phys_addr
	}

	fn walk_page_table_debug(
		include_tracking_flags: bool,
		callback: &mut dyn FnMut(&PageTableEntryDebug<'_>) -> bool,
	) {
		let flag_mask = if include_tracking_flags {
			PageTableEntryFlags::all()
		} else {
			PageTableEntryFlags::all() - PageTableEntryFlags::DIRTY - PageTableEntryFlags::ACCESSED
		};

		fn dump(
			virtual_address: usize,
			entry_size: usize,
			depth: usize,
			node: &[AtomicU64; 512],
			callback: &mut dyn FnMut(&PageTableEntryDebug<'_>) -> bool,
			flag_mask: PageTableEntryFlags,
		) {
			for (index, x) in node.iter().enumerate() {
				let entry = entry_from_raw(x.load(Relaxed));
				let flags = entry.flags() & flag_mask;
				let is_present = flags.contains(PageTableEntryFlags::PRESENT);
				let has_children = is_present
					&& !flags.contains(PageTableEntryFlags::HUGE_PAGE)
					&& entry_size > SIZE_4KIB.usize();
				let visit_children = callback(&PageTableEntryDebug {
					physical_addr: entry.addr().as_u64() as usize,
					virtual_addr: virtual_address + entry_size * index,
					size: entry_size,
					flags: flags.bits(),
					flags_debug: &flags,
					depth,
					has_children,
					is_present,
				});

				if visit_children {
					assert!(has_children);
					dump(
						virtual_address + entry_size * index,
						entry_size >> 9,
						depth + 1,
						to_child(node, index),
						callback,
						flag_mask,
					);
				}
			}
		}
		dump(0, 1 << 39, 0, table_root_node(), callback, flag_mask);
	}
}

pub struct NumIdentityPage(usize);

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
