use alloc::boxed::Box;
use alloc::vec::Vec;

use crate::{arch, env};
// The generic interface simply uses all default implementations of the
// SyscallInterface trait.
pub struct Generic;
impl Generic {
	pub fn init(&self) {
		// Interface-specific initialization steps.
	}

	pub fn get_application_parameters(&self) -> (i32, *const *const u8, *const *const u8) {
		let mut argv = Vec::new();

		let name = Box::leak(Box::new("bin\0")).as_ptr();
		argv.push(name);

		let args = env::args();
		debug!("Setting argv as: {args:?}");
		for arg in args {
			let ptr = Box::leak(format!("{arg}\0").into_boxed_str()).as_ptr();
			argv.push(ptr);
		}

		let mut envv = Vec::new();

		let envs = env::vars();
		debug!("Setting envv as: {envs:?}");
		for (key, value) in envs {
			let ptr = Box::leak(format!("{key}={value}\0").into_boxed_str()).as_ptr();
			envv.push(ptr);
		}
		envv.push(core::ptr::null::<u8>());

		let argc = argv.len() as i32;
		let argv = argv.leak().as_ptr();
		// do we have more than a end marker? If not, return as null pointer
		let envv = if envv.len() == 1 {
			core::ptr::null::<*const u8>()
		} else {
			envv.leak().as_ptr()
		};

		(argc, argv, envv)
	}

	pub fn shutdown(&self, error_code: i32) -> ! {
		// This is a stable message used for detecting exit codes for different hypervisors.
		panic_println!("exit status {error_code}");

		arch::processor::shutdown(error_code)
	}
}
