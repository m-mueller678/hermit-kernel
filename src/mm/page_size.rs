use crate::logging::format_binary_si_bytes;
use crate::{Arch, PagingTrait};

#[derive(Clone, Copy, Eq, PartialEq, PartialOrd, Ord)]
pub struct PageSize(pub usize);

impl core::fmt::Debug for PageSize {
	fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
		core::fmt::Display::fmt(&format_binary_si_bytes(self.usize()), f)
	}
}
impl core::fmt::Display for PageSize {
	fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
		core::fmt::Display::fmt(&format_binary_si_bytes(self.usize()), f)
	}
}

impl PageSize {
	pub fn usize(self) -> usize {
		self.0
	}

	pub const fn from_exponent(x: u32) -> usize {
		usize::checked_shl(1, x).unwrap()
	}

	pub fn log2(self) -> u32 {
		self.0.trailing_zeros()
	}

	fn low_mask(self) -> usize {
		self.usize() - 1
	}
}
impl core::ops::Mul<usize> for PageSize {
	type Output = usize;

	fn mul(self, rhs: usize) -> Self::Output {
		self.0 * rhs
	}
}
impl core::ops::Mul<PageSize> for usize {
	type Output = usize;

	fn mul(self, rhs: PageSize) -> Self::Output {
		self * rhs.0
	}
}

impl core::ops::Div<PageSize> for usize {
	type Output = usize;

	fn div(self, rhs: PageSize) -> Self::Output {
		self >> rhs.log2()
	}
}

impl PageSize {
	pub fn lesser(self) -> Option<Self> {
		Arch::lesser_page_size(self)
	}
	pub fn greater(self) -> Option<Self> {
		Arch::greater_page_size(self)
	}
	pub fn size_index(self) -> usize {
		Arch::page_size_index(self)
	}
	pub fn ps_div_ceil(self, x: usize) -> usize {
		let up = x & self.low_mask() != 0;
		(x >> self.log2()) + up as usize
	}
	pub fn ps_align_down(self, x: usize) -> usize {
		x & (usize::MAX << self.log2())
	}
	pub fn ps_align_up(self, x: usize) -> usize {
		let mask = self.low_mask();
		if x & mask == 0 { x } else { (x + mask) & !mask }
	}
}
