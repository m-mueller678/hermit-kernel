use core::{slice, str};

use log::Level;

/// Writes to the kernel log.
/// # Safety
/// target and message must form valid &str values.
#[hermit_macro::system]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sys_log(
	level: usize,
	target_ptr: *const u8,
	target_len: usize,
	message_ptr: *const u8,
	message_len: usize,
) {
	let level = match level {
		1 => Level::Error,
		2 => Level::Warn,
		3 => Level::Info,
		4 => Level::Debug,
		5 => Level::Trace,
		_ => panic!("invalid log level"),
	};
	// SAFETY: Caller ensures this is a valid slice
	let target = unsafe { slice::from_raw_parts(target_ptr, target_len) };
	// SAFETY: Caller ensures this is valid utf8
	let target = unsafe { str::from_utf8_unchecked(target) };
	// SAFETY: Caller ensures this is a valid slice
	let message = unsafe { slice::from_raw_parts(message_ptr, message_len) };
	// SAFETY: Caller ensures this is valid utf8
	let message = unsafe { str::from_utf8_unchecked(message) };
	crate::logging::KERNEL_LOGGER.log_common(level, target, &format_args!("{message}"));
}
