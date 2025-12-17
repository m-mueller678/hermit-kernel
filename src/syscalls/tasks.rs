use crate::config::USER_STACK_SIZE;
use crate::errno::Errno;
use crate::scheduler;
use crate::scheduler::task::{Priority, TaskId};

pub type Tid = i32;

fn exit(arg: i32) -> ! {
	debug!("Exit program with error code {arg}!");
	super::shutdown(arg)
}

#[hermit_macro::system]
#[unsafe(no_mangle)]
pub extern "C" fn sys_exit(status: i32) -> ! {
	exit(status)
}

#[hermit_macro::system]
#[unsafe(no_mangle)]
pub extern "C" fn sys_abort() -> ! {
	exit(-1)
}

#[hermit_macro::system]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sys_spawn2(
	func: unsafe extern "C" fn(usize),
	arg: usize,
	prio: u8,
	stack_size: usize,
	selector: isize,
) -> Tid {
	unsafe { scheduler::spawn(func, arg, Priority::from(prio), stack_size, selector).into() }
}

#[hermit_macro::system]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sys_spawn(
	id: *mut Tid,
	func: unsafe extern "C" fn(usize),
	arg: usize,
	prio: u8,
	selector: isize,
) -> i32 {
	let new_id = unsafe {
		scheduler::spawn(func, arg, Priority::from(prio), USER_STACK_SIZE, selector).into()
	};

	if !id.is_null() {
		unsafe {
			*id = new_id;
		}
	}

	0
}

#[hermit_macro::system]
#[unsafe(no_mangle)]
pub extern "C" fn sys_join(id: Tid) -> i32 {
	match scheduler::join(TaskId::from(id)) {
		Ok(()) => 0,
		_ => -i32::from(Errno::Inval),
	}
}
