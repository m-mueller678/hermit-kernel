use core::alloc::Layout;
use core::ptr::NonNull;
use core::sync::atomic::{AtomicBool, AtomicU32};

use pci_types::BaseClass;
use x86_64::VirtAddr;
use x86_64::structures::paging::{PageSize, Size4KiB};

use crate::arch::TaskTrait;
use crate::scheduler::task::tls::TlsBuilder;

pub struct Task(NonNull<TaskHeader>);

pub struct TaskHeader {}

impl TaskTrait for Task {
	fn create(
		main: fn(*const u8),
		main_arg: *const u8,
		min_stack_size: usize,
		min_interrupt_stack_size: usize,
	) -> Self {
		let tls_builder = TlsBuilder::from_env();
		let tls_layout = tls_builder
			.as_ref()
			.map(|x| x.layout())
			.unwrap_or(Layout::new::<()>());
		let layout = Layout::new::<()>();
		let (layout, task_header_offset) = layout.extend(Layout::new::<TaskHeader>()).unwrap();
		let (layout, tls_offset) = layout.extend(tls_layout).unwrap();
		todo!()
	}

	fn switch(self, from_interrupt_handler: bool) -> Self {
		todo!()
	}
}

pub struct Stacks {
	/// start address of allocated virtual memory region
	virt_addr: VirtAddr,
	/// total size of all stacks
	task_stack_size: usize,
	interrupt_stack_size: usize,
}

const PAGE_SIZE: usize = Size4KiB::SIZE as usize;
const NMI_STACK_SIZE: usize = PAGE_SIZE;

fn new(task_stack_size: usize, interrupt_stack_size: usize) -> Stacks {
	let phys_size = task_stack_size + interrupt_stack_size + NMI_STACK_SIZE;
	let virt_size = phys_size + 4 * PAGE_SIZE;

	let layout = PageLayout::from_size(total_size + 4 * BasePageSize::SIZE as usize).unwrap();
	let page_range = PageAlloc::allocate(layout).unwrap();
	let virt_addr = VirtAddr::from(page_range.start());

	let frame_layout = PageLayout::from_size(total_size).unwrap();
	let frame_range = FrameAlloc::allocate(frame_layout)
		.expect("Failed to allocate Physical Memory for TaskStacks");
	let phys_addr = PhysAddr::from(frame_range.start());

	debug!(
		"Create stacks at {:p} with a size of {} KB",
		virt_addr,
		total_size >> 10
	);

	let mut flags = PageTableEntryFlags::empty();
	flags.normal().writable().execute_disable();

	// map IST1 into the address space
	crate::arch::mm::paging::map::<BasePageSize>(
		virt_addr + BasePageSize::SIZE,
		phys_addr,
		IST_SIZE / BasePageSize::SIZE as usize,
		flags,
	);

	// map kernel stack into the address space
	crate::arch::mm::paging::map::<BasePageSize>(
		virt_addr + IST_SIZE + 2 * BasePageSize::SIZE,
		phys_addr + IST_SIZE,
		DEFAULT_STACK_SIZE / BasePageSize::SIZE as usize,
		flags,
	);

	// map user stack into the address space
	crate::arch::mm::paging::map::<BasePageSize>(
		virt_addr + IST_SIZE + DEFAULT_STACK_SIZE + 3 * BasePageSize::SIZE,
		phys_addr + IST_SIZE + DEFAULT_STACK_SIZE,
		user_stack_size / BasePageSize::SIZE as usize,
		flags,
	);

	// clear user stack
	unsafe {
		ptr::write_bytes(
			(virt_addr + IST_SIZE + DEFAULT_STACK_SIZE + 3 * BasePageSize::SIZE).as_mut_ptr::<u8>(),
			0,
			user_stack_size,
		);
	}

	TaskStacks::Common(CommonStack {
		virt_addr,
		phys_addr,
		total_size,
	})
}
