use crate::arch::get_processor_count;
use crate::time::cpu_timestamp_frequency_mhz;

/// Returns the number of processors currently online.
#[hermit_macro::system]
#[unsafe(no_mangle)]
pub extern "C" fn sys_get_processor_count() -> usize {
	get_processor_count().try_into().unwrap()
}

#[hermit_macro::system]
#[unsafe(no_mangle)]
pub extern "C" fn sys_available_parallelism() -> usize {
	get_processor_count().try_into().unwrap()
}

/// Returns the processor frequency in MHz.
#[hermit_macro::system]
#[unsafe(no_mangle)]
pub extern "C" fn sys_get_processor_frequency() -> u16 {
	cpu_timestamp_frequency_mhz().try_into().unwrap()
}
