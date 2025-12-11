pub struct Scheduler {
	/// Scheduler for this CPU Core.
	scheduler: Cell<*mut PerCoreScheduler>,
	/// Queues to handle incoming requests from the other cores
	#[cfg(feature = "smp")]
	pub scheduler_input: InterruptTicketMutex<SchedulerInput>,
}

impl Scheduler {
	pub const fn new() -> Self {
		Scheduler {
			scheduler: Cell::new(ptr::null_mut()),
			#[cfg(feature = "smp")]
			scheduler_input: InterruptTicketMutex::new(SchedulerInput::new()),
		}
	}
	pub fn spawn_task(
		&self,
		func: unsafe extern "C" fn(*mut u8),
		arg: *mut u8,
		prio: Priority,
		core_id: Option<CoreId>,
		// if this is None, this is a future task
		// future tasks are non-preemptible and have their main function called each time they are ready
		stack_size: Option<TaskStacks>,
	) -> TaskId {
		todo!()
	}
	pub fn exit_current_task(&self, exit_code: i32) {
		todo!()
	}
	pub fn unblock_task(task: TaskId) {
		todo!()
	}
	pub fn block_current_task(until: Option<NonZeroU64>) {
		todo!()
	}
	pub fn get_current_task_id() -> TaskId {
		todo!()
	}
}
