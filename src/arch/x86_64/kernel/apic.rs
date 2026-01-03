use alloc::alloc::alloc;
use alloc::vec::Vec;
use core::alloc::Layout;
use core::arch::x86_64::_mm_mfence;
#[cfg(feature = "acpi")]
use core::fmt;
use core::num::NonZeroUsize;
use core::sync::atomic::Ordering;
use core::{cmp, ptr};

use arch::x86_64::kernel::core_local::*;
use arch::x86_64::kernel::{interrupts, processor};
use hermit_sync::{OnceCell, SpinMutex, without_interrupts};
use x86_64::registers::control::Cr3;
use x86_64::registers::model_specific::Msr;

use super::interrupts::IDT;
use crate::arch::PageFlagsTrait;
use crate::arch::x86_64::SIZE_4KIB;
use crate::arch::x86_64::kernel::CURRENT_STACK_ADDRESS;
use crate::config::*;
use crate::mm::{map_contiguous, virtual_memory};
use crate::scheduler::CoreId;
use crate::time::{cpu_timestamp_frequency_mhz, cpu_timestamp_us};
use crate::{PageFlags, arch, env};

/// APIC Location and Status (R/W) See Table 35-2. See Section 10.4.4, Local APIC  Status and Location.
const IA32_APIC_BASE: Msr = Msr::new(0x1b);

/// TSC Target of Local APIC s TSC Deadline Mode (R/W)  See Table 35-2
const IA32_TSC_DEADLINE: Msr = Msr::new(0x6e0);

/// x2APIC Task Priority register (R/W)
const IA32_X2APIC_TPR: u32 = 0x808;

/// x2APIC End of Interrupt. If ( CPUID.01H:ECX.\[bit 21\]  = 1 )
const IA32_X2APIC_EOI: u32 = 0x80b;

/// x2APIC Spurious Interrupt Vector register (R/W)
const IA32_X2APIC_SIVR: u32 = 0x80f;

/// Error Status Register. If ( CPUID.01H:ECX.\[bit 21\]  = 1 )
const IA32_X2APIC_ESR: u32 = 0x828;

/// x2APIC Interrupt Command register (R/W)
const IA32_X2APIC_ICR: u32 = 0x830;

/// x2APIC LVT Timer Interrupt register (R/W)
const IA32_X2APIC_LVT_TIMER: u32 = 0x832;

/// x2APIC LVT Thermal Sensor Interrupt register (R/W)
const IA32_X2APIC_LVT_THERMAL: u32 = 0x833;

/// x2APIC LVT Performance Monitor register (R/W)
const IA32_X2APIC_LVT_PMI: u32 = 0x834;

/// If ( CPUID.01H:ECX.\[bit 21\]  = 1 )
const IA32_X2APIC_LVT_LINT0: u32 = 0x835;

/// If ( CPUID.01H:ECX.\[bit 21\]  = 1 )
const IA32_X2APIC_LVT_LINT1: u32 = 0x836;

/// If ( CPUID.01H:ECX.\[bit 21\]  = 1 )
const IA32_X2APIC_LVT_ERROR: u32 = 0x837;

/// x2APIC Initial Count register (R/W)
const IA32_X2APIC_INIT_COUNT: u32 = 0x838;

/// x2APIC Current Count register (R/O)
const IA32_X2APIC_CUR_COUNT: u32 = 0x839;

/// x2APIC Divide Configuration register (R/W)
const IA32_X2APIC_DIV_CONF: u32 = 0x83e;

const MP_FLT_SIGNATURE: u32 = 0x5f50_4d5f;
const MP_CONFIG_SIGNATURE: u32 = 0x504d_4350;

const APIC_ICR2: usize = 0x0310;

const APIC_DIV_CONF_DIVIDE_BY_8: u64 = 0b0010;
const APIC_EOI_ACK: u64 = 0;
const APIC_ICR_DELIVERY_MODE_FIXED: u64 = 0x000;
const APIC_ICR_DELIVERY_MODE_INIT: u64 = 0x500;
const APIC_ICR_DELIVERY_MODE_STARTUP: u64 = 0x600;
const APIC_ICR_DELIVERY_STATUS_PENDING: u32 = 1 << 12;
const APIC_ICR_LEVEL_TRIGGERED: u64 = 1 << 15;
const APIC_ICR_LEVEL_ASSERT: u64 = 1 << 14;
const APIC_LVT_MASK: u64 = 1 << 16;
const APIC_LVT_TIMER_TSC_DEADLINE: u64 = 1 << 18;
const APIC_SIVR_ENABLED: u64 = 1 << 8;

/// Register index: ID
#[allow(dead_code)]
const IOAPIC_REG_ID: u32 = 0x0000;
/// Register index: version
const IOAPIC_REG_VER: u32 = 0x0001;
/// Redirection table base
const IOAPIC_REG_TABLE: u32 = 0x0010;

const TLB_FLUSH_INTERRUPT_NUMBER: u8 = 112;
const WAKEUP_INTERRUPT_NUMBER: u8 = 121;
pub const TIMER_INTERRUPT_NUMBER: u8 = 123;
const ERROR_INTERRUPT_NUMBER: u8 = 126;
const SPURIOUS_INTERRUPT_NUMBER: u8 = 127;

/// Physical and virtual memory address for our SMP boot code.
///
/// While our boot processor is already in x86-64 mode, application processors boot up in 16-bit real mode
/// and need an address in the CS:IP addressing scheme to jump to.
/// The CS:IP addressing scheme is limited to 2^20 bytes (= 1 MiB).
const SMP_BOOT_CODE_ADDRESS: usize = 0x8000;

const SMP_BOOT_CODE_OFFSET_ENTRY: usize = 0x08;
const SMP_BOOT_CODE_OFFSET_CPU_ID: usize = SMP_BOOT_CODE_OFFSET_ENTRY + 0x08;
const SMP_BOOT_CODE_OFFSET_PML4: usize = SMP_BOOT_CODE_OFFSET_CPU_ID + 0x04;

const X2APIC_ENABLE: u64 = 1 << 10;

/// IO apic virtual address
static IOAPIC_ADDRESS: OnceCell<NonZeroUsize> = OnceCell::new();

/// Stores the Local APIC IDs of all CPUs. The index equals the Core ID.
/// Both numbers often match, but don't need to (e.g. when a core has been disabled).
static CPU_LOCAL_APIC_IDS: SpinMutex<Vec<u8>> = SpinMutex::new(Vec::new());

/// After calibration, initialize the APIC Timer with this counter value to let it fire an interrupt
/// after 1 microsecond.
static CALIBRATED_COUNTER_VALUE: OnceCell<u64> = OnceCell::new();

/// MP Floating Pointer Structure
#[repr(C, packed)]
struct ApicMP {
	signature: u32,
	mp_config: u32,
	length: u8,
	version: u8,
	checksum: u8,
	features: [u8; 5],
}

/// MP Configuration Table
#[repr(C, packed)]
struct ApicConfigTable {
	signature: u32,
	length: u16,
	revision: u8,
	checksum: u8,
	oem_id: [u8; 8],
	product_id: [u8; 12],
	oem_table: u32,
	oem_table_size: u16,
	entry_count: u16,
	lapic: u32,
	extended_table_length: u16,
	extended_table_checksum: u8,
	reserved: u8,
}

/// APIC Processor Entry
#[repr(C, packed)]
struct ApicProcessorEntry {
	ty: u8,
	id: u8,
	version: u8,
	cpu_flags: u8,
	cpu_signature: u32,
	cpu_feature: u32,
	reserved: [u32; 2],
}

/// IO APIC Entry
#[repr(C, packed)]
struct ApicIoEntry {
	ty: u8,
	id: u8,
	version: u8,
	enabled: u8,
	addr: u32,
}

#[cfg(feature = "acpi")]
#[repr(C, packed)]
struct AcpiMadtHeader {
	local_apic_address: u32,
	flags: u32,
}

#[cfg(feature = "acpi")]
#[repr(C, packed)]
struct AcpiMadtRecordHeader {
	entry_type: u8,
	length: u8,
}

#[cfg(feature = "acpi")]
#[repr(C, packed)]
struct ProcessorLocalApicRecord {
	acpi_processor_id: u8,
	apic_id: u8,
	flags: u32,
}

#[cfg(feature = "acpi")]
impl fmt::Display for ProcessorLocalApicRecord {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "{{ acpi_processor_id: {}, ", { self.acpi_processor_id })?;
		write!(f, "apic_id: {}, ", { self.apic_id })?;
		write!(f, "flags: {} }}", { self.flags })?;
		Ok(())
	}
}

#[cfg(feature = "acpi")]
const CPU_FLAG_ENABLED: u32 = 1 << 0;

#[cfg(feature = "acpi")]
#[repr(C, packed)]
struct IoApicRecord {
	id: u8,
	reserved: u8,
	address: u32,
	global_system_interrupt_base: u32,
}

#[cfg(feature = "acpi")]
impl fmt::Display for IoApicRecord {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "{{ id: {}, ", { self.id })?;
		write!(f, "reserved: {}, ", { self.reserved })?;
		write!(f, "address: {:#X}, ", { self.address })?;
		write!(f, "global_system_interrupt_base: {} }}", {
			self.global_system_interrupt_base
		})?;
		Ok(())
	}
}

extern "x86-interrupt" fn tlb_flush_handler(_stack_frame: interrupts::ExceptionStackFrame) {
	debug!("Received TLB Flush Interrupt");
	increment_irq_counter(TLB_FLUSH_INTERRUPT_NUMBER);
	let (frame, val) = Cr3::read_raw();
	unsafe {
		Cr3::write_raw(frame, val);
	}
	eoi();
}

extern "x86-interrupt" fn error_interrupt_handler(stack_frame: interrupts::ExceptionStackFrame) {
	long_panic!(
		("APIC LVT Error Interrupt"),
		("ESR: {:#X}", local_apic_read(IA32_X2APIC_ESR)),
		("{stack_frame:#?}"),
	);
}

extern "x86-interrupt" fn spurious_interrupt_handler(stack_frame: interrupts::ExceptionStackFrame) {
	panic!("Spurious Interrupt: {stack_frame:#?}");
}

extern "x86-interrupt" fn wakeup_handler(_stack_frame: interrupts::ExceptionStackFrame) {
	use crate::scheduler::PerCoreSchedulerExt;

	debug!("Received Wakeup Interrupt");
	increment_irq_counter(WAKEUP_INTERRUPT_NUMBER);
	let core_scheduler = core_scheduler();
	core_scheduler.check_input();
	eoi();
	if core_scheduler.is_scheduling() {
		core_scheduler.reschedule();
	}
}

#[inline]
pub fn add_local_apic_id(id: u8) {
	CPU_LOCAL_APIC_IDS.lock().push(id);
}

pub fn local_apic_id_count() -> u32 {
	CPU_LOCAL_APIC_IDS.lock().len() as u32
}

fn init_ioapic_address(phys_addr: usize) {
	if env::is_uefi() {
		// UEFI systems have already id mapped everything, so we can just set the physical address as the virtual one
		IOAPIC_ADDRESS
			.set(NonZeroUsize::new(phys_addr).unwrap())
			.unwrap();
	} else {
		let ioapic_address =
			virtual_memory::allocate(SIZE_4KIB, NonZeroUsize::new(1).unwrap()).unwrap();
		IOAPIC_ADDRESS.set(ioapic_address).unwrap();
		debug!(
			"Mapping IOAPIC at {phys_addr:p} to virtual address {ioapic_address:p}",
			phys_addr = ptr::without_provenance::<u8>(phys_addr),
			ioapic_address = ptr::without_provenance::<u8>(ioapic_address.get())
		);

		unsafe {
			map_contiguous(
				SIZE_4KIB,
				ioapic_address,
				phys_addr,
				NonZeroUsize::new(1).unwrap(),
				PageFlags::device().writable(),
			);
		}
	}
}

fn default_apic() -> usize {
	let default_address = 0xfee0_0000;
	warn!(
		"Using default APIC address: {:p}",
		ptr::without_provenance::<u8>(default_address)
	);
	init_ioapic_address(default_address);
	default_address
}

pub fn eoi() {
	local_apic_write(IA32_X2APIC_EOI, APIC_EOI_ACK);
}

pub fn init() {
	init_x2apic();

	// Set gates to ISRs for the APIC interrupts we are going to enable.
	unsafe {
		let mut idt = IDT.lock();
		idt[ERROR_INTERRUPT_NUMBER]
			.set_handler_fn(error_interrupt_handler)
			.set_stack_index(0);
		idt[SPURIOUS_INTERRUPT_NUMBER]
			.set_handler_fn(spurious_interrupt_handler)
			.set_stack_index(0);
		{
			idt[TLB_FLUSH_INTERRUPT_NUMBER]
				.set_handler_fn(tlb_flush_handler)
				.set_stack_index(0);
			interrupts::add_irq_name(TLB_FLUSH_INTERRUPT_NUMBER - 32, "TLB flush");
			idt[WAKEUP_INTERRUPT_NUMBER]
				.set_handler_fn(wakeup_handler)
				.set_stack_index(0);
			interrupts::add_irq_name(WAKEUP_INTERRUPT_NUMBER - 32, "Wakeup");
		}
	}

	// Initialize interrupt handling over APIC.
	// All interrupts of the PIC have already been masked, so it doesn't need to be disabled again.
	init_local_apic();

	if !processor::supports_tsc_deadline() {
		// We have an older APIC Timer without TSC Deadline support, which has a maximum timeout
		// and needs to be calibrated.
		calibrate_timer();
	}

	// initialize IO-APIC
	init_ioapic();
}

fn init_ioapic() {
	let max_entry = ioapic_max_redirection_entry() + 1;
	info!("IOAPIC v{} has {} entries", ioapic_version(), max_entry);

	// now lets turn everything else on
	for i in 0..max_entry {
		// Turn off the Programmable Interrupt Timer Interrupt (IRQ 0) and
		// the Real Time Clock (IRQ 2).
		let enabled = !matches!(i, 0 | 2);
		ioapic_set_interrupt(i, 0, enabled);
	}
}

fn ioapic_set_interrupt(irq: u8, apicid: u8, enabled: bool) {
	assert!(irq <= 24);

	let off = u32::from(irq * 2);
	let ioredirect_upper = u32::from(apicid) << 24;
	let mut ioredirect_lower = u32::from(0x20 + irq);
	if !enabled {
		debug!("Disabling irq {irq}");
		ioredirect_lower |= 1 << 16;
	}

	ioapic_write(IOAPIC_REG_TABLE + off, ioredirect_lower);
	ioapic_write(IOAPIC_REG_TABLE + off + 1, ioredirect_upper);
}

pub fn init_local_apic() {
	// Mask out all interrupts we don't need right now.
	local_apic_write(IA32_X2APIC_LVT_TIMER, APIC_LVT_MASK);
	local_apic_write(IA32_X2APIC_LVT_THERMAL, APIC_LVT_MASK);
	local_apic_write(IA32_X2APIC_LVT_PMI, APIC_LVT_MASK);
	local_apic_write(IA32_X2APIC_LVT_LINT0, APIC_LVT_MASK);
	local_apic_write(IA32_X2APIC_LVT_LINT1, APIC_LVT_MASK);

	// Set the interrupt number of the Error interrupt.
	local_apic_write(IA32_X2APIC_LVT_ERROR, u64::from(ERROR_INTERRUPT_NUMBER));

	// allow all interrupts
	local_apic_write(IA32_X2APIC_TPR, 0x00);

	// Finally, enable the Local APIC by setting the interrupt number for spurious interrupts
	// and providing the enable bit.
	local_apic_write(
		IA32_X2APIC_SIVR,
		APIC_SIVR_ENABLED | (u64::from(SPURIOUS_INTERRUPT_NUMBER)),
	);
}

fn calibrate_timer() {
	// The APIC Timer is used to provide a one-shot interrupt for the tickless timer
	// implemented through processor::get_timer_ticks.
	// Therefore determine a counter value for 1 microsecond, which is the resolution
	// used throughout all of Hermit. Wait 30ms for accuracy.
	let microseconds = 30_000;

	// Be sure that all interrupts for calibration accuracy and initialize the counter are disabled.
	// Dividing the counter value by 8 still provides enough accuracy for 1 microsecond resolution,
	// but allows for longer timeouts than a smaller divisor.
	// For example, on an Intel Xeon E5-2650 v3 @ 2.30GHz, the counter is usually calibrated to
	// 125, which allows for timeouts of approximately 34 seconds (u32::MAX / 125).

	local_apic_write(IA32_X2APIC_DIV_CONF, APIC_DIV_CONF_DIVIDE_BY_8);
	local_apic_write(IA32_X2APIC_INIT_COUNT, u64::from(u32::MAX));

	// Wait until the calibration time has elapsed.
	processor::udelay(microseconds);

	// Save the difference of the initial value and current value as the result of the calibration
	// and re-enable interrupts.
	let calibrated_counter_value =
		(u64::from(u32::MAX - local_apic_read(IA32_X2APIC_CUR_COUNT))) / microseconds;
	CALIBRATED_COUNTER_VALUE
		.set(calibrated_counter_value)
		.unwrap();
	debug!(
		"Calibrated APIC Timer with a counter value of {calibrated_counter_value} for 1 microsecond",
	);
}

fn __set_oneshot_timer(wakeup_time: Option<u64>) {
	if let Some(wt) = wakeup_time {
		if processor::supports_tsc_deadline() {
			// wt is the absolute wakeup time in microseconds based on processor::get_timer_ticks.
			// We can simply multiply it by the processor frequency to get the absolute Time-Stamp Counter deadline
			// (see processor::get_timer_ticks).
			let tsc_deadline = wt * cpu_timestamp_frequency_mhz();

			// Enable the APIC Timer in TSC-Deadline Mode and let it start by writing to the respective MSR.
			local_apic_write(
				IA32_X2APIC_LVT_TIMER,
				APIC_LVT_TIMER_TSC_DEADLINE | u64::from(TIMER_INTERRUPT_NUMBER),
			);
			let mut ia32_tsc_deadline = IA32_TSC_DEADLINE;
			unsafe {
				ia32_tsc_deadline.write(tsc_deadline);
			}
		} else {
			// Calculate the relative timeout from the absolute wakeup time.
			// Maintain a minimum value of one tick, otherwise the timer interrupt does not fire at all.
			// The Timer Counter Register is also a 32-bit register, which we must not overflow for longer timeouts.
			let current_time = cpu_timestamp_us();
			let ticks = if wt > current_time {
				wt - current_time
			} else {
				1
			};
			let init_count = cmp::min(
				CALIBRATED_COUNTER_VALUE.get().unwrap() * ticks,
				u64::from(u32::MAX),
			);

			// Enable the APIC Timer in One-Shot Mode and let it start by setting the initial counter value.
			local_apic_write(IA32_X2APIC_LVT_TIMER, u64::from(TIMER_INTERRUPT_NUMBER));
			local_apic_write(IA32_X2APIC_INIT_COUNT, init_count);
		}
	} else {
		// Disable the APIC Timer.
		local_apic_write(IA32_X2APIC_LVT_TIMER, APIC_LVT_MASK);
	}
}

pub fn set_oneshot_timer(wakeup_time: Option<u64>) {
	without_interrupts(|| {
		__set_oneshot_timer(wakeup_time);
	});
}

pub fn init_x2apic() {
	debug!("Enable x2APIC support");
	// The CPU supports the modern x2APIC mode, which uses MSRs for communication.
	// Enable it.
	let mut msr = IA32_APIC_BASE;
	let mut apic_base = unsafe { msr.read() };
	apic_base |= X2APIC_ENABLE;
	unsafe {
		msr.write(apic_base);
	}
}

/// Initialize the required _start variables for the next CPU to be booted.
pub fn init_next_processor_variables() {
	// Allocate stack for the CPU and pass the addresses.
	let layout = Layout::from_size_align(KERNEL_STACK_SIZE, SIZE_4KIB.usize()).unwrap();
	let stack = unsafe { alloc(layout) };
	assert!(!stack.is_null());
	CURRENT_STACK_ADDRESS.store(stack, Ordering::Relaxed);
}

/// Boot all Application Processors
/// This algorithm is derived from Intel MultiProcessor Specification 1.4, B.4, but testing has shown
/// that a second STARTUP IPI and setting the BIOS Reset Vector are no longer necessary.
/// This is partly confirmed by <https://wiki.osdev.org/Symmetric_Multiprocessing>
pub fn boot_application_processors() {
	use core::hint;

	use hermit_entry::boot_info::RawBootInfo;

	use super::start;

	let smp_boot_code = include_bytes!(concat!(core::env!("OUT_DIR"), "/boot.bin"));

	// We shouldn't have any problems fitting the boot code into a single page, but let's better be sure.
	assert!(
		smp_boot_code.len() <= SIZE_4KIB.usize(),
		"SMP Boot Code is larger than a page"
	);
	debug!("SMP boot code is {} bytes long", smp_boot_code.len());
	unsafe {
		ptr::copy_nonoverlapping(
			smp_boot_code.as_ptr(),
			ptr::with_exposed_provenance_mut::<u8>(SMP_BOOT_CODE_ADDRESS),
			smp_boot_code.len(),
		);
	}

	unsafe {
		let (frame, val) = Cr3::read_raw();
		let value = frame.start_address().as_u64() | u64::from(val);
		// Pass the PML4 page table address to the boot code.
		*(ptr::with_exposed_provenance_mut::<u32>(
			SMP_BOOT_CODE_ADDRESS + SMP_BOOT_CODE_OFFSET_PML4,
		)) = value.try_into().unwrap();
		// Set entry point
		debug!(
			"Set entry point for application processor to {:p}",
			start::_start as *const ()
		);
		ptr::with_exposed_provenance_mut::<
			unsafe extern "C" fn(Option<&'static RawBootInfo>, cpu_id: u32) -> !,
		>(SMP_BOOT_CODE_ADDRESS + SMP_BOOT_CODE_OFFSET_ENTRY)
		.write_unaligned(start::_start);
	}

	// Now wake up each application processor.
	let apic_ids = CPU_LOCAL_APIC_IDS.lock();
	let core_id = core_id();

	for (core_id_to_boot, &apic_id) in apic_ids.iter().enumerate() {
		let core_id_to_boot = core_id_to_boot as u32;
		if core_id_to_boot != core_id {
			unsafe {
				ptr::with_exposed_provenance_mut::<u32>(
					SMP_BOOT_CODE_ADDRESS + SMP_BOOT_CODE_OFFSET_CPU_ID,
				)
				.write(core_id_to_boot);
			}
			let destination = u64::from(apic_id) << 32;

			debug!("Waking up CPU {core_id_to_boot} with Local APIC ID {apic_id}");
			init_next_processor_variables();

			// Save the current number of initialized CPUs.
			let current_processor_count = arch::get_processor_count();

			// Send an INIT IPI.
			local_apic_write(
				IA32_X2APIC_ICR,
				destination
					| APIC_ICR_LEVEL_TRIGGERED
					| APIC_ICR_LEVEL_ASSERT
					| APIC_ICR_DELIVERY_MODE_INIT,
			);
			processor::udelay(200);

			local_apic_write(
				IA32_X2APIC_ICR,
				destination | APIC_ICR_LEVEL_TRIGGERED | APIC_ICR_DELIVERY_MODE_INIT,
			);
			processor::udelay(10000);

			// Send a STARTUP IPI.
			local_apic_write(
				IA32_X2APIC_ICR,
				destination
					| APIC_ICR_DELIVERY_MODE_STARTUP
					| ((SMP_BOOT_CODE_ADDRESS as u64) >> 12),
			);
			debug!("Waiting for it to respond");

			// Wait until the application processor has finished initializing.
			// It will indicate this by counting up cpu_online.
			while current_processor_count == arch::get_processor_count() {
				hint::spin_loop();
			}
		}
	}

	print_information();
}

pub fn ipi_tlb_flush() {
	if arch::get_processor_count() > 1 {
		let apic_ids = CPU_LOCAL_APIC_IDS.lock();
		let core_id = core_id();

		// Ensure that all memory operations have completed before issuing a TLB flush.
		unsafe {
			_mm_mfence();
		}

		// Send an IPI with our TLB Flush interrupt number to all other CPUs.
		without_interrupts(|| {
			for (core_id_to_interrupt, &apic_id) in apic_ids.iter().enumerate() {
				if core_id_to_interrupt != usize::try_from(core_id).unwrap() {
					let destination = u64::from(apic_id) << 32;
					local_apic_write(
						IA32_X2APIC_ICR,
						destination
							| APIC_ICR_LEVEL_ASSERT
							| APIC_ICR_DELIVERY_MODE_FIXED
							| u64::from(TLB_FLUSH_INTERRUPT_NUMBER),
					);
				}
			}
		});
	}
}

/// Send an inter-processor interrupt to wake up a CPU Core that is in a HALT state.
#[allow(unused_variables)]
pub fn wakeup_core(core_id_to_wakeup: CoreId) {
	#[cfg(not(feature = "idle-poll"))]
	if core_id_to_wakeup != core_id()
		&& !crate::processor::supports_mwait()
		&& crate::scheduler::take_core_hlt_state(core_id_to_wakeup)
	{
		without_interrupts(|| {
			let apic_ids = CPU_LOCAL_APIC_IDS.lock();
			let local_apic_id = apic_ids[core_id_to_wakeup as usize];
			let destination = u64::from(local_apic_id) << 32;
			local_apic_write(
				IA32_X2APIC_ICR,
				destination
					| APIC_ICR_LEVEL_ASSERT
					| APIC_ICR_DELIVERY_MODE_FIXED
					| u64::from(WAKEUP_INTERRUPT_NUMBER),
			);
		});
	}
}

fn local_apic_read(x2apic_msr: u32) -> u32 {
	unsafe { Msr::new(x2apic_msr).read() as u32 }
}

fn ioapic_write(reg: u32, value: u32) {
	unsafe {
		let base = ptr::with_exposed_provenance_mut::<u32>(IOAPIC_ADDRESS.get().unwrap().get());
		base.write_volatile(reg);
		base.add(4).write_volatile(value);
	}
}

fn ioapic_read(reg: u32) -> u32 {
	unsafe {
		let base = ptr::with_exposed_provenance_mut::<u32>(IOAPIC_ADDRESS.get().unwrap().get());
		base.write_volatile(reg);
		base.add(4).read_volatile()
	}
}

fn ioapic_version() -> u32 {
	ioapic_read(IOAPIC_REG_VER) & 0xff
}

fn ioapic_max_redirection_entry() -> u8 {
	((ioapic_read(IOAPIC_REG_VER) >> 16) & 0xff) as u8
}

fn local_apic_write(x2apic_msr: u32, value: u64) {
	unsafe {
		Msr::new(x2apic_msr).write(value);
	}
}

pub fn print_information() {
	infoheader!(" MULTIPROCESSOR INFORMATION ");
	infoentry!("APIC in use", "x2APIC");
	infoentry!("Initialized CPUs", arch::get_processor_count());
	infofooter!();
}
