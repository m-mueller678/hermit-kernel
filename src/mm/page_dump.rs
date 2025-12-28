use core::ptr;

use arrayvec::ArrayString;

use crate::Arch;
use crate::arch::PagingTrait;
use crate::logging::format_binary_si_bytes;

pub fn dump_page_table_hierarchical() {
	Arch::walk_page_table_debug(true, &mut |entry| {
		if entry.is_present {
			let indent = entry.depth * 2;
			let size = format_binary_si_bytes(entry.size);
			let va = ptr::without_provenance::<u8>(entry.virtual_addr);
			let pa = ptr::without_provenance::<u8>(entry.physical_addr);
			let flags = entry.flags_debug;
			println!("{:indent$}Page {size:3}@{va:16p}->{pa:16p} {flags:?}", "");
		}
		entry.has_children
	});
}

pub fn dump_page_table_leaves() {
	let mut flags_formatted = ArrayString::<256>::new();
	let mut virtual_start = 0;
	let mut virtual_end = 0;
	let mut page_size = 1;
	let mut flags = 0;
	macro_rules! print_current {
		() => {
			if virtual_end != virtual_start {
				let size = virtual_end - virtual_start;
				let page_count = size / page_size;
				let size = format_binary_si_bytes(size);
				let page_size = format_binary_si_bytes(page_size);
				let start = ptr::without_provenance::<u8>(virtual_start);
				let end = ptr::without_provenance::<u8>(virtual_end);
				println!(
					"{start:16p}..{end:16p} ({page_count:4}x{page_size:4} = {size:4}): {flags_formatted:?}"
				);
			}
		};
	}
	Arch::walk_page_table_debug(false, &mut |entry| {
		use core::fmt::Write;
		if entry.has_children {
			return true;
		}
		assert_eq!(virtual_end, entry.virtual_addr);
		if entry.flags == flags && entry.size == page_size {
			virtual_end = entry.virtual_addr + entry.size;
		} else {
			print_current!();
			virtual_start = entry.virtual_addr;
			virtual_end = entry.virtual_addr + entry.size;
			page_size = entry.size;
			flags = entry.flags;
			flags_formatted.clear();
			write!(flags_formatted, "{:?}", entry.flags_debug).unwrap();
		}
		false
	});
	print_current!();
}
