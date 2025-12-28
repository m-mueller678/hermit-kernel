use core::ptr;
use core::sync::atomic::{AtomicPtr, AtomicU32, Ordering};

use hermit_entry::boot_info::{PlatformInfo, RawBootInfo};
use memory_addresses::PhysAddr;
use x86_64::registers::control::{Cr0, Cr4};

use crate::arch::x86_64::kernel::core_local::*;
use crate::env::{self};

#[cfg(feature = "acpi")]
pub mod acpi;
pub mod apic;
pub mod core_local;
pub mod gdt;
pub mod interrupts;
#[cfg(feature = "kernel-stack")]
pub mod kernel_stack;
#[cfg(feature = "pci")]
pub mod pci;
pub mod pic;
pub mod pit;
pub mod processor;
pub mod scheduler;
pub mod serial;
mod start;
pub mod switch;
pub(crate) mod systemtime;
#[cfg(feature = "vga")]
pub mod vga;

pub fn get_ram_address() -> PhysAddr {
	PhysAddr::new(env::boot_info().hardware_info.phys_addr_range.start)
}

pub fn get_image_size() -> usize {
	let range = &env::boot_info().load_info.kernel_image_addr_range;
	(range.end - range.start) as usize
}

pub fn get_processor_count() -> u32 {
	CPU_ONLINE.load(Ordering::Acquire)
}

pub fn args() -> Option<&'static str> {
	match env::boot_info().platform_info {
		PlatformInfo::Multiboot { command_line, .. }
		| PlatformInfo::LinuxBootParams { command_line, .. } => command_line,
		_ => None,
	}
}

/// Real Boot Processor initialization as soon as we have put the first Welcome message on the screen.
pub fn boot_processor_init() {
	processor::detect_features();
	processor::configure();

	if cfg!(feature = "vga") {
		#[cfg(feature = "vga")]
		vga::init();
	}

	crate::mm::init();
	CoreLocal::get().add_irq_counter();
	env::init();
	gdt::add_current_core();
	interrupts::load_idt();
	pic::init();

	crate::time::init_cpu_timestamp_frequency();
	crate::logging::KERNEL_LOGGER.set_time(true);
	processor::print_information();
	debug!("Cr0 = {:?}", Cr0::read());
	debug!("Cr4 = {:?}", Cr4::read());
	interrupts::install();
	systemtime::init();

	#[cfg(feature = "acpi")]
	acpi::init();
	#[cfg(feature = "pci")]
	pci::init();

	apic::init();
	scheduler::install_timer_handler();
}

/// Application Processor initialization
pub fn application_processor_init() {
	CoreLocal::install();
	processor::configure();
	gdt::add_current_core();
	interrupts::load_idt();
	apic::init_x2apic();
	apic::init_local_apic();
	debug!("Cr0 = {:?}", Cr0::read());
	debug!("Cr4 = {:?}", Cr4::read());
}

pub fn boot_next_processor() {
	// This triggers apic::boot_application_processors (bare-metal/QEMU)
	// to initialize the next processor.
	let cpu_online = CPU_ONLINE.fetch_add(1, Ordering::Release);

	if cpu_online == 0 {
		apic::boot_application_processors();
	}
}

pub fn print_statistics() {
	interrupts::print_statistics();
}

/// `CPU_ONLINE` is the count of CPUs that finished initialization.
///
/// It also synchronizes initialization of CPU cores.
pub static CPU_ONLINE: AtomicU32 = AtomicU32::new(0);

pub static CURRENT_STACK_ADDRESS: AtomicPtr<u8> = AtomicPtr::new(ptr::null_mut());

#[inline(never)]
#[unsafe(no_mangle)]
unsafe extern "C" fn pre_init(boot_info: Option<&'static RawBootInfo>, cpu_id: u32) -> ! {
	use x86_64::registers::control::Cr0Flags;

	// Enable caching
	unsafe {
		Cr0::update(|flags| flags.remove(Cr0Flags::CACHE_DISABLE | Cr0Flags::NOT_WRITE_THROUGH));
	}

	if cpu_id == 0 {
		env::set_boot_info(*boot_info.unwrap());

		crate::boot_processor_main()
	} else {
		crate::application_processor_main();
	}
}
