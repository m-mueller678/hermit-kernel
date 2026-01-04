use crate::logging::format_binary_si_bytes;
use crate::{Arch, PagingTrait};

#[derive(Clone, Copy, Eq, PartialEq, PartialOrd, Ord)]
pub struct PageSize(pub usize);

impl core::fmt::Debug for PageSize {
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
}
impl core::ops::Mul<usize> for PageSize {
	type Output = usize;

	fn mul(self, rhs: usize) -> Self::Output {
		self.0 * rhs
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
}
