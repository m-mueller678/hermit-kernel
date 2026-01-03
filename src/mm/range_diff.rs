use core::ops::Range;

use align_address::Align;
use arrayvec::ArrayVec;

use crate::MIN_PAGE_SIZE;
use crate::logging::{format_addr, format_binary_si_bytes};

const CAPACITY: usize = 64;

pub struct RangeDiff {
	// first memory_count ranges are available memory ranges, rest are holes.
	ranges: ArrayVec<Range<usize>, CAPACITY>,
	memory_count: usize,
	i_memory: usize,
	// index of first hole that may inersect memory range `i_memory`.
	// All ranges before this are known not to intersect.
	i_hole: usize,
}

impl RangeDiff {
	pub fn new(
		memories: impl Iterator<Item = Range<usize>>,
		holes: impl Iterator<Item = Range<usize>>,
	) -> Self {
		fn range_non_empty(x: &Range<usize>) -> bool {
			x.start < x.end
		}
		let size = MIN_PAGE_SIZE.usize();
		let mut ret = RangeDiff {
			ranges: ArrayVec::new(),
			memory_count: 0,
			i_memory: 0,
			i_hole: 0,
		};
		ret.ranges.extend(
			memories
				.map(|x| x.start.align_up(size)..x.end.align_down(size))
				.filter(range_non_empty),
		);
		ret.memory_count = ret.ranges.len();
		ret.ranges.extend(
			holes
				.map(|x| x.start.align_down(size)..x.end.align_up(size))
				.filter(range_non_empty),
		);
		ret.ranges[..ret.memory_count].sort_unstable_by_key(|x| x.start);
		ret.ranges[ret.memory_count..].sort_unstable_by_key(|x| x.start);
		for [a, b] in ret.ranges[..ret.memory_count].array_windows::<2>() {
			assert!(a.end <= b.start);
		}
		ret.i_hole = ret.memory_count;
		for x in &mut ret.ranges[..ret.memory_count] {
			info!(
				"available physical memory region: {:016p}..{:016p}",
				format_addr(x.start),
				format_addr(x.end)
			);
		}
		for x in &ret.ranges[ret.memory_count..] {
			info!(
				"reserved physical memory region:  {:016p}..{:016p}",
				format_addr(x.start),
				format_addr(x.end)
			);
		}
		ret
	}

	fn advance_to_size(&mut self, size: usize) -> Option<Range<usize>> {
		loop {
			if self.i_memory == self.memory_count {
				return None;
			}
			let memory = &self.ranges[self.i_memory];
			let start = memory.start;
			let end = if let Some(hole) = self.ranges.get(self.i_hole) {
				memory.end.min(hole.start)
			} else {
				memory.end
			};
			if start + size <= end {
				self.ranges[self.i_memory].start = start + size;
				return Some(start..end);
			} else {
				if start != end {
					warn!(
						"discarding {} of physical memory, too small for requested early allocation",
						format_binary_si_bytes(end - start)
					);
				}
				if let Some(hole) = self.ranges.get(self.i_hole).cloned() {
					let memory_start = &mut self.ranges[self.i_memory].start;
					if *memory_start < hole.end {
						*memory_start = hole.end;
					}
					self.i_hole += 1;
				} else {
					self.i_hole = self.memory_count;
					self.i_memory += 1;
				}
				continue;
			}
		}
	}

	pub fn take_fixed(&mut self, size: usize) -> Option<usize> {
		let found_range = self.advance_to_size(size)?;
		self.ranges[self.i_memory].start += size;
		Some(found_range.start)
	}

	pub fn take_remaining(&mut self) -> Option<Range<usize>> {
		let found_range = self.advance_to_size(1)?;
		self.ranges[self.i_memory].start = found_range.end;
		Some(found_range)
	}

	pub fn memory_end(&self) -> usize {
		self.ranges[self.memory_count - 1].end
	}
}
