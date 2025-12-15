use alloc::alloc::Global;

use insert_take::{InsertState, InsertTake};

use crate::core_local::{CoreLocal, core_scheduler};
use crate::logging::Microseconds;
use crate::processor;

struct Common {
	time: u32,
	thread: u16,
}

struct LogEntry {
	common: Common,
	event: Event,
}

impl Common {
	fn new() -> Self {
		Common {
			time: processor::get_timer_ticks() as u32,
			thread: core_scheduler().get_current_task_id().into() as u16,
		}
	}
}

#[derive(Debug)]
pub enum Event {
	FutexWait { address: usize, is_set: bool },
	FutexWake { address: usize, woken: u32 },
}

#[must_use]
impl Event {
	pub fn submit(self) {
		LOG.insert(
			&mut CoreLocal::get().debug_trace_local.borrow_mut().0,
			LogEntry {
				common: Common::new(),
				event: self,
			},
		);
	}
}

pub struct DebugTraceLocal(InsertState<LogEntry, Global, 1000>);

impl DebugTraceLocal {
	pub fn new() -> Self {
		DebugTraceLocal(InsertState::new())
	}
}

static LOG: InsertTake<LogEntry, Global, 1000> = InsertTake::new();

pub fn dump() {
	let mut log: alloc::vec::Vec<_> = LOG.take().into_iter().collect();
	log.sort_unstable_by_key(|x| x.common.time);
	dbg!(log.len());
	for LogEntry {
		common: Common { time, thread },
		event,
	} in log
	{
		let time = Microseconds(time.into());
		println!("debug_trace {time} {thread} {event:?}");
	}
}
