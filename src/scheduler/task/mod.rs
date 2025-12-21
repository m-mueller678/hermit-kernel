#![allow(clippy::type_complexity)]

pub(crate) mod tls;

use alloc::collections::LinkedList;
use alloc::rc::Rc;
use core::cell::RefCell;
use core::num::NonZeroU64;
use core::{cmp, fmt};

use memory_addresses::VirtAddr;

use self::tls::Tls;
use crate::arch::core_local::*;
use crate::arch::scheduler::TaskStacks;
use crate::scheduler::CoreId;
use crate::{Arch, ArchTrait, arch};

/// Returns the most significant bit.
///
/// # Examples
///
/// ```
/// assert_eq!(msb(0), None);
/// assert_eq!(msb(1), 0);
/// assert_eq!(msb(u64::MAX), 63);
/// ```
#[inline]
fn msb(n: u64) -> Option<u32> {
	NonZeroU64::new(n).map(|n| u64::BITS - 1 - n.leading_zeros())
}

/// The status of the task - used for scheduling
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub(crate) enum TaskStatus {
	Invalid,
	Ready,
	Running,
	Blocked,
	Finished,
	Idle,
}

/// Unique identifier for a task (i.e. `pid`).
#[derive(PartialEq, Eq, PartialOrd, Ord, Debug, Clone, Copy)]
pub struct TaskId(i32);

impl TaskId {
	pub const fn into(self) -> i32 {
		self.0
	}

	pub const fn from(x: i32) -> Self {
		TaskId(x)
	}
}

impl fmt::Display for TaskId {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "{}", self.0)
	}
}

/// Priority of a task
#[derive(PartialEq, Eq, PartialOrd, Ord, Debug, Clone, Copy)]
pub struct Priority(u8);

impl Priority {
	pub const fn into(self) -> u8 {
		self.0
	}

	pub const fn from(x: u8) -> Self {
		Priority(x)
	}
}

impl fmt::Display for Priority {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "{}", self.0)
	}
}

#[allow(dead_code)]
pub const HIGH_PRIO: Priority = Priority::from(3);
pub const NORMAL_PRIO: Priority = Priority::from(2);
#[allow(dead_code)]
pub const LOW_PRIO: Priority = Priority::from(1);
pub const IDLE_PRIO: Priority = Priority::from(0);

/// Maximum number of priorities
pub const NO_PRIORITIES: usize = 31;

#[derive(Copy, Clone, Debug)]
pub(crate) struct TaskHandle {
	id: TaskId,
	#[allow(dead_code)]
	priority: Priority,
	core_id: CoreId,
}

impl TaskHandle {
	pub fn new(id: TaskId, priority: Priority, core_id: CoreId) -> Self {
		Self {
			id,
			priority,
			core_id,
		}
	}

	pub fn get_core_id(&self) -> CoreId {
		self.core_id
	}

	pub fn get_id(&self) -> TaskId {
		self.id
	}
}

impl Ord for TaskHandle {
	fn cmp(&self, other: &Self) -> cmp::Ordering {
		self.id.cmp(&other.id)
	}
}

impl PartialOrd for TaskHandle {
	fn partial_cmp(&self, other: &Self) -> Option<cmp::Ordering> {
		Some(self.cmp(other))
	}
}

impl PartialEq for TaskHandle {
	fn eq(&self, other: &Self) -> bool {
		self.id == other.id
	}
}

impl Eq for TaskHandle {}

/// Realize a priority queue for tasks
pub(crate) struct PriorityTaskQueue {
	queues: [LinkedList<Rc<RefCell<Task>>>; NO_PRIORITIES],
	prio_bitmap: u64,
}

impl PriorityTaskQueue {
	/// Creates an empty priority queue for tasks
	pub const fn new() -> PriorityTaskQueue {
		const EMPTY_LIST: LinkedList<Rc<RefCell<Task>>> = LinkedList::new();
		PriorityTaskQueue {
			queues: [EMPTY_LIST; NO_PRIORITIES],
			prio_bitmap: 0,
		}
	}

	/// Add a task by its priority to the queue
	pub fn push(&mut self, task: Rc<RefCell<Task>>) {
		let i = task.borrow().prio.into() as usize;
		//assert!(i < NO_PRIORITIES, "Priority {} is too high", i);

		self.prio_bitmap |= (1 << i) as u64;
		let queue = &mut self.queues[i];
		queue.push_back(task);
	}

	fn pop_from_queue(&mut self, queue_index: usize) -> Option<Rc<RefCell<Task>>> {
		let task = self.queues[queue_index].pop_front();
		if self.queues[queue_index].is_empty() {
			self.prio_bitmap &= !(1 << queue_index as u64);
		}

		task
	}

	/// Returns true if the queue is empty.
	pub fn is_empty(&self) -> bool {
		self.prio_bitmap == 0
	}

	/// Returns reference to prio_bitmap
	#[allow(dead_code)]
	#[inline]
	pub fn get_priority_bitmap(&self) -> &u64 {
		&self.prio_bitmap
	}

	/// Pop the task with the highest priority from the queue
	pub fn pop(&mut self) -> Option<Rc<RefCell<Task>>> {
		if let Some(i) = msb(self.prio_bitmap) {
			return self.pop_from_queue(i as usize);
		}

		None
	}

	/// Pop the next task, which has a higher or the same priority as `prio`
	pub fn pop_with_prio(&mut self, prio: Priority) -> Option<Rc<RefCell<Task>>> {
		if let Some(i) = msb(self.prio_bitmap)
			&& i >= u32::from(prio.into())
		{
			return self.pop_from_queue(i as usize);
		}

		None
	}

	/// Returns the highest priority of all available task
	#[cfg(any(target_arch = "x86_64", target_arch = "riscv64"))]
	pub fn get_highest_priority(&self) -> Priority {
		if let Some(i) = msb(self.prio_bitmap) {
			Priority::from(i.try_into().unwrap())
		} else {
			IDLE_PRIO
		}
	}
}

/// A task control block, which identifies either a process or a thread
#[cfg_attr(any(target_arch = "x86_64", target_arch = "aarch64"), repr(align(128)))]
#[cfg_attr(
	not(any(target_arch = "x86_64", target_arch = "aarch64")),
	repr(align(64))
)]
pub(crate) struct Task {
	/// The ID of this context
	pub id: TaskId,
	/// Status of a task, e.g. if the task is ready or blocked
	pub status: TaskStatus,
	/// Task priority,
	pub prio: Priority,
	/// Last stack pointer before a context switch to another task
	pub last_stack_pointer: VirtAddr,
	/// Last stack pointer on the user stack before jumping to kernel space
	pub user_stack_pointer: VirtAddr,
	/// Last FPU state before a context switch to another task using the FPU
	pub last_fpu_state: arch::processor::FPUState,
	/// ID of the core this task is running on
	pub core_id: CoreId,
	/// Stack of the task
	pub stacks: TaskStacks,
	/// Task Thread-Local-Storage (TLS)
	pub tls: Option<Tls>,
}

pub(crate) trait TaskFrame {
	/// Create the initial stack frame for a new task
	fn create_stack_frame(&mut self, func: unsafe extern "C" fn(usize), arg: usize);
}

impl Task {
	pub fn new(
		tid: TaskId,
		core_id: CoreId,
		task_status: TaskStatus,
		task_prio: Priority,
		stacks: TaskStacks,
	) -> Task {
		debug!("Creating new task {tid} on core {core_id}");

		Task {
			id: tid,
			status: task_status,
			prio: task_prio,
			last_stack_pointer: VirtAddr::zero(),
			user_stack_pointer: VirtAddr::zero(),
			last_fpu_state: arch::processor::FPUState::new(),
			core_id,
			stacks,
			tls: None,
		}
	}

	pub fn new_idle(tid: TaskId, core_id: CoreId) -> Task {
		debug!("Creating idle task {tid}");
		Task {
			id: tid,
			status: TaskStatus::Idle,
			prio: IDLE_PRIO,
			last_stack_pointer: VirtAddr::zero(),
			user_stack_pointer: VirtAddr::zero(),
			last_fpu_state: arch::processor::FPUState::new(),
			core_id,
			stacks: TaskStacks::from_boot_stacks(),
			tls: None,
		}
	}
}

/*impl Drop for Task {
	fn drop(&mut self) {
		debug!("Drop task {}", self.id);
	}
}*/

struct BlockedTask {
	task: Rc<RefCell<Task>>,
	wakeup_time: Option<u64>,
}

impl BlockedTask {
	pub fn new(task: Rc<RefCell<Task>>, wakeup_time: Option<u64>) -> Self {
		Self { task, wakeup_time }
	}
}

pub(crate) struct BlockedTaskQueue {
	list: LinkedList<BlockedTask>,
}

impl BlockedTaskQueue {
	pub const fn new() -> Self {
		Self {
			list: LinkedList::new(),
		}
	}

	fn mark_ready(task: &RefCell<Task>) {
		let mut borrowed = task.borrow_mut();
		debug!(
			"Waking up task {} on core {}",
			borrowed.id, borrowed.core_id
		);

		assert!(
			borrowed.core_id == core_id(),
			"Try to wake up task {} on the wrong core {} != {}",
			borrowed.id,
			borrowed.core_id,
			core_id()
		);

		assert!(
			borrowed.status == TaskStatus::Blocked,
			"Trying to wake up task {} which is not blocked",
			borrowed.id
		);
		borrowed.status = TaskStatus::Ready;
	}

	/// Blocks the given task for `wakeup_time` ticks, or indefinitely if None is given.
	pub fn add(&mut self, task: Rc<RefCell<Task>>, wakeup_time: Option<u64>) {
		{
			// Set the task status to Blocked.
			let mut borrowed = task.borrow_mut();
			debug!("Blocking task {}", borrowed.id);

			assert_eq!(
				borrowed.status,
				TaskStatus::Running,
				"Trying to block task {} which is not running",
				borrowed.id
			);
			borrowed.status = TaskStatus::Blocked;
		}

		let new_node = BlockedTask::new(task, wakeup_time);

		// Shall the task automatically be woken up after a certain time?
		if let Some(wt) = wakeup_time {
			let mut cursor = self.list.cursor_front_mut();
			let set_oneshot_timer = || {
				Arch::set_oneshot_timer(wakeup_time);
			};

			while let Some(node) = cursor.current() {
				let node_wakeup_time = node.wakeup_time;
				if node_wakeup_time.is_none() || wt < node_wakeup_time.unwrap() {
					cursor.insert_before(new_node);

					set_oneshot_timer();
					return;
				}

				cursor.move_next();
			}

			set_oneshot_timer();
		}

		self.list.push_back(new_node);
	}

	/// Manually wake up a blocked task.
	pub fn custom_wakeup(&mut self, task: TaskHandle) -> Rc<RefCell<Task>> {
		let mut first_task = true;
		let mut cursor = self.list.cursor_front_mut();

		// Loop through all blocked tasks to find it.
		while let Some(node) = cursor.current() {
			if node.task.borrow().id == task.get_id() {
				// Remove it from the list of blocked tasks.
				let task_ref = node.task.clone();
				cursor.remove_current();

				// If this is the first task, adjust the One-Shot Timer to fire at the
				// next task's wakeup time (if any).
				if first_task {
					Arch::set_oneshot_timer(
						cursor
							.current()
							.map_or_else(|| None, |node| node.wakeup_time),
					);
				}

				// Wake it up.
				Self::mark_ready(&task_ref);

				return task_ref;
			}

			first_task = false;
			cursor.move_next();
		}

		unreachable!();
	}

	/// Wakes up all tasks whose wakeup time has elapsed.
	///
	/// Should be called by the One-Shot Timer interrupt handler when the wakeup time for
	/// at least one task has elapsed.
	pub fn handle_waiting_tasks(&mut self, ready_queue: &mut PriorityTaskQueue) {
		// Get the current time.
		let time = arch::processor::get_timer_ticks();

		// Get the wakeup time of this task and check if we have reached the first task
		// that hasn't elapsed yet or waits indefinitely.
		// This iterator has to be consumed to actually remove the elements.
		let newly_ready_tasks = self.list.extract_if(|blocked_task| {
			blocked_task
				.wakeup_time
				.is_some_and(|wakeup_time| wakeup_time < time)
		});

		for task in newly_ready_tasks {
			Self::mark_ready(&task.task);
			ready_queue.push(task.task);
		}

		let new_task_wakeup_time = self.list.front().and_then(|task| task.wakeup_time);
		let network_wakeup_time = None;
		let timer_wakeup_time = match (new_task_wakeup_time, network_wakeup_time) {
			(None, None) => None,
			(None, Some(network_wt)) => Some(network_wt),
			(Some(task_wt), None) => Some(task_wt),
			(Some(task_wt), Some(network_wt)) => Some(u64::min(task_wt, network_wt)),
		};

		Arch::set_oneshot_timer(timer_wakeup_time);
	}
}
