use memory_addresses::{PhysAddr, VirtAddr};

use crate::mm::physicalmem::{allocate_physical, deallocate_physical};
use crate::mm::virtualmem::{allocate_virtual, deallocate_virtual};

/// Allocate physical memory.
#[hermit_macro::system]
#[unsafe(no_mangle)]
pub extern "C" fn sys_allocate_physical(size: usize, align: usize) -> usize {
	match allocate_physical(size, align) {
		Ok(x) => {
			assert!(!x.is_null());
			x.as_usize()
		}
		Err(_) => 0,
	}
}

/// Deallocate physical memory previously allocated with [sys_allocate_physical].
#[hermit_macro::system]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sys_deallocate_physical(addr: usize, size: usize) {
	unsafe { deallocate_physical(PhysAddr::from(addr), size) };
}

/// Allocate virtual memory.
#[hermit_macro::system]
#[unsafe(no_mangle)]
pub extern "C" fn sys_allocate_virtual(size: usize, align: usize) -> usize {
	match allocate_virtual(size, align) {
		Ok(x) => {
			assert!(!x.is_null());
			x.as_usize()
		}
		Err(_) => 0,
	}
}

/// Deallocate virtual memory previously allocated with [sys_allocate_virtual].
#[hermit_macro::system]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sys_deallocate_virtual(addr: usize, size: usize) {
	unsafe { deallocate_virtual(VirtAddr::from(addr), size) };
}

#[hermit_macro::system]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sys_global_tlb_flush() {
	cfg_if::cfg_if!(
		if #[cfg(target_arch = "x86_64")]{
			// #[cfg(feature="smp")]
			// crate::arch::x86_64::kernel::apic::ipi_tlb_flush();
			x86_64::structures::paging::mapper::MapperFlushAll::new().flush_all();
		}else{
			unimplemented!();
		}
	);
}
