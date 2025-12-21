#![allow(clippy::type_complexity)]

use alloc::boxed::Box;
use alloc::collections::{BTreeMap, VecDeque};
use alloc::rc::Rc;
use alloc::vec::Vec;
use core::cell::RefCell;
use core::ptr;
#[cfg(target_arch = "x86_64")]
use core::sync::atomic::AtomicBool;
use core::sync::atomic::{AtomicI32, AtomicU32, Ordering};

use crossbeam_utils::Backoff;
use hermit_sync::*;
#[cfg(target_arch = "riscv64")]
use riscv::register::sstatus;

use crate::arch::core_local::*;
use crate::arch::get_processor_count;
#[cfg(target_arch = "riscv64")]
use crate::arch::switch::switch_to_task;
#[cfg(target_arch = "x86_64")]
use crate::arch::switch::{switch_to_fpu_owner, switch_to_task};
use crate::kernel::scheduler::TaskStacks;
use crate::scheduler::task::*;
use crate::{Arch, ArchTrait};

pub mod task;

static NO_TASKS: AtomicU32 = AtomicU32::new(0);
/// Map between Core ID and per-core scheduler
static SCHEDULER_INPUTS: SpinMutex<Vec<&InterruptTicketMutex<SchedulerInput>>> =
	SpinMutex::new(Vec::new());
#[cfg(target_arch = "x86_64")]
static CORE_HLT_STATE: SpinMutex<Vec<&AtomicBool>> = SpinMutex::new(Vec::new());
/// Map between Task ID and Queue of waiting tasks
static WAITING_TASKS: InterruptTicketMutex<BTreeMap<TaskId, VecDeque<TaskHandle>>> =
	InterruptTicketMutex::new(BTreeMap::new());
/// Map between Task ID and TaskHandle
static TASKS: InterruptTicketMutex<BTreeMap<TaskId, TaskHandle>> =
	InterruptTicketMutex::new(BTreeMap::new());

/// Unique identifier for a core.
pub type CoreId = u32;

pub(crate) struct SchedulerInput {
	/// Queue of new tasks
	new_tasks: VecDeque<NewTask>,
	/// Queue of task, which are wakeup by another core
	wakeup_tasks: VecDeque<TaskHandle>,
}

impl SchedulerInput {
	pub fn new() -> Self {
		Self {
			new_tasks: VecDeque::new(),
			wakeup_tasks: VecDeque::new(),
		}
	}
}

#[cfg_attr(any(target_arch = "x86_64", target_arch = "aarch64"), repr(align(128)))]
#[cfg_attr(
	not(any(target_arch = "x86_64", target_arch = "aarch64")),
	repr(align(64))
)]
pub(crate) struct PerCoreScheduler {
	/// Core ID of this per-core scheduler
	core_id: CoreId,
	/// Task which is currently running
	current_task: Rc<RefCell<Task>>,
	/// Idle Task
	idle_task: Rc<RefCell<Task>>,
	/// Task that currently owns the FPU
	#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
	fpu_owner: Rc<RefCell<Task>>,
	/// Queue of tasks, which are ready
	ready_queue: PriorityTaskQueue,
	/// Queue of tasks, which are finished and can be released
	finished_tasks: VecDeque<Rc<RefCell<Task>>>,
	/// Queue of blocked tasks, sorted by wakeup time.
	blocked_tasks: BlockedTaskQueue,
}

pub(crate) trait PerCoreSchedulerExt {
	/// Triggers the scheduler to reschedule the tasks.
	/// Interrupt flag will be cleared during the reschedule
	fn reschedule(self);

	/// Terminate the current task on the current core.
	fn exit(self, exit_code: i32) -> !;
}

impl PerCoreSchedulerExt for &mut PerCoreScheduler {
	#[cfg(target_arch = "x86_64")]
	fn reschedule(self) {
		without_interrupts(|| {
			if let Some(last_stack_pointer) = self.scheduler() {
				let (new_stack_pointer, is_idle) = {
					let borrowed = self.current_task.borrow();
					(
						borrowed.last_stack_pointer,
						borrowed.status == TaskStatus::Idle,
					)
				};

				if is_idle || Rc::ptr_eq(&self.current_task, &self.fpu_owner) {
					unsafe {
						switch_to_fpu_owner(
							last_stack_pointer,
							new_stack_pointer.as_u64() as usize,
						);
					}
				} else {
					unsafe {
						switch_to_task(last_stack_pointer, new_stack_pointer.as_u64() as usize);
					}
				}
			}
		});
	}

	/// Trigger an interrupt to reschedule the system
	#[cfg(target_arch = "aarch64")]
	fn reschedule(self) {
		use aarch64_cpu::asm::barrier::{NSH, SY, dsb, isb};
		use arm_gic::IntId;
		use arm_gic::gicv3::{GicV3, SgiTarget, SgiTargetGroup};

		use crate::interrupts::SGI_RESCHED;

		dsb(NSH);
		isb(SY);

		let reschedid = IntId::sgi(SGI_RESCHED.into());
		let core_id = self.core_id;

		GicV3::send_sgi(
			reschedid,
			SgiTarget::List {
				affinity3: 0,
				affinity2: 0,
				affinity1: 0,
				target_list: 1 << core_id,
			},
			SgiTargetGroup::CurrentGroup1,
		);

		interrupts::enable();
	}

	#[cfg(target_arch = "riscv64")]
	fn reschedule(self) {
		without_interrupts(|| self.scheduler());
	}

	fn exit(self, exit_code: i32) -> ! {
		without_interrupts(|| {
			// Get the current task.
			let mut current_task_borrowed = self.current_task.borrow_mut();
			assert_ne!(
				current_task_borrowed.status,
				TaskStatus::Idle,
				"Trying to terminate the idle task"
			);

			// Finish the task and reschedule.
			debug!(
				"Finishing task {} with exit code {}",
				current_task_borrowed.id, exit_code
			);
			current_task_borrowed.status = TaskStatus::Finished;
			NO_TASKS.fetch_sub(1, Ordering::SeqCst);

			let current_id = current_task_borrowed.id;
			drop(current_task_borrowed);

			// wakeup tasks, which are waiting for task with the identifier id
			if let Some(mut queue) = WAITING_TASKS.lock().remove(&current_id) {
				while let Some(task) = queue.pop_front() {
					self.custom_wakeup(task);
				}
			}
		});

		self.reschedule();
		unreachable!()
	}
}

struct NewTask {
	tid: TaskId,
	func: unsafe extern "C" fn(usize),
	arg: usize,
	prio: Priority,
	core_id: CoreId,
	stacks: TaskStacks,
}

impl From<NewTask> for Task {
	fn from(value: NewTask) -> Self {
		let NewTask {
			tid,
			func,
			arg,
			prio,
			core_id,
			stacks,
		} = value;
		let mut task = Self::new(tid, core_id, TaskStatus::Ready, prio, stacks);
		task.create_stack_frame(func, arg);
		task
	}
}

impl PerCoreScheduler {
	/// Spawn a new task.
	pub unsafe fn spawn(
		func: unsafe extern "C" fn(usize),
		arg: usize,
		prio: Priority,
		core_id: CoreId,
		stack_size: usize,
	) -> TaskId {
		// Create the new task.
		let tid = get_tid();
		let stacks = TaskStacks::new(stack_size);
		let new_task = NewTask {
			tid,
			func,
			arg,
			prio,
			core_id,
			stacks,
		};

		// Add it to the task lists.
		let wakeup = {
			let mut input_locked = get_scheduler_input(core_id).lock();
			WAITING_TASKS.lock().insert(tid, VecDeque::with_capacity(1));
			TASKS
				.lock()
				.insert(tid, TaskHandle::new(tid, prio, core_id));
			NO_TASKS.fetch_add(1, Ordering::SeqCst);

			if core_id == core_scheduler().core_id {
				let task = Rc::new(RefCell::new(Task::from(new_task)));
				core_scheduler().ready_queue.push(task);
				false
			} else {
				input_locked.new_tasks.push_back(new_task);
				true
			}
		};

		debug!("Creating task {tid} with priority {prio} on core {core_id}");

		if wakeup {
			Arch::wakeup_core(core_id);
		}

		tid
	}

	/// Returns `true` if a reschedule is required
	#[inline]
	#[cfg(any(target_arch = "x86_64", target_arch = "riscv64"))]
	pub fn is_scheduling(&self) -> bool {
		self.current_task.borrow().prio < self.ready_queue.get_highest_priority()
	}

	#[inline]
	pub fn handle_waiting_tasks(&mut self) {
		without_interrupts(|| {
			self.blocked_tasks
				.handle_waiting_tasks(&mut self.ready_queue);
		});
	}

	pub fn custom_wakeup(&mut self, task: TaskHandle) {
		if task.get_core_id() == self.core_id {
			without_interrupts(|| {
				let task = self.blocked_tasks.custom_wakeup(task);
				self.ready_queue.push(task);
			});
		} else {
			get_scheduler_input(task.get_core_id())
				.lock()
				.wakeup_tasks
				.push_back(task);
			// Wake up the CPU
			Arch::wakeup_core(task.get_core_id());
		}
	}

	#[inline]
	pub fn block_current_task(&mut self, wakeup_time: Option<u64>) {
		without_interrupts(|| {
			self.blocked_tasks
				.add(self.current_task.clone(), wakeup_time);
		});
	}

	#[inline]
	pub fn get_current_task_handle(&self) -> TaskHandle {
		without_interrupts(|| {
			let current_task_borrowed = self.current_task.borrow();

			TaskHandle::new(
				current_task_borrowed.id,
				current_task_borrowed.prio,
				current_task_borrowed.core_id,
			)
		})
	}

	#[inline]
	pub fn get_current_task_id(&self) -> TaskId {
		without_interrupts(|| self.current_task.borrow().id)
	}

	/// Returns reference to prio_bitmap
	#[allow(dead_code)]
	#[inline]
	pub fn get_priority_bitmap(&self) -> &u64 {
		self.ready_queue.get_priority_bitmap()
	}

	#[cfg(target_arch = "x86_64")]
	pub fn set_current_kernel_stack(&self) {
		let current_task_borrowed = self.current_task.borrow();
		let tss = unsafe { &mut *CoreLocal::get().tss.get() };

		let rsp = current_task_borrowed.stacks.get_kernel_stack()
			+ current_task_borrowed.stacks.get_kernel_stack_size() as u64
			- TaskStacks::MARKER_SIZE as u64;
		tss.privilege_stack_table[0] = rsp.into();
		CoreLocal::get().kernel_stack.set(rsp.as_mut_ptr());
		let ist_start = current_task_borrowed.stacks.get_interrupt_stack()
			+ current_task_borrowed.stacks.get_interrupt_stack_size() as u64
			- TaskStacks::MARKER_SIZE as u64;
		tss.interrupt_stack_table[0] = ist_start.into();
	}

	#[cfg(target_arch = "riscv64")]
	pub fn set_current_kernel_stack(&self) {
		let current_task_borrowed = self.current_task.borrow();

		let stack = (current_task_borrowed.stacks.get_kernel_stack()
			+ current_task_borrowed.stacks.get_kernel_stack_size() as u64
			- TaskStacks::MARKER_SIZE as u64)
			.as_u64();
		CoreLocal::get().kernel_stack.set(stack);
	}

	/// Save the FPU context for the current FPU owner and restore it for the current task,
	/// which wants to use the FPU now.
	#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
	pub fn fpu_switch(&mut self) {
		if !Rc::ptr_eq(&self.current_task, &self.fpu_owner) {
			debug!(
				"Switching FPU owner from task {} to {}",
				self.fpu_owner.borrow().id,
				self.current_task.borrow().id
			);

			self.fpu_owner.borrow_mut().last_fpu_state.save();
			self.current_task.borrow().last_fpu_state.restore();
			self.fpu_owner = self.current_task.clone();
		}
	}

	/// Check if a finished task could be deleted.
	fn cleanup_tasks(&mut self) {
		// Pop the first finished task and remove it from the TASKS list, which implicitly deallocates all associated memory.
		while let Some(finished_task) = self.finished_tasks.pop_front() {
			debug!("Cleaning up task {}", finished_task.borrow().id);
		}
	}

	pub fn check_input(&mut self) {
		let mut input_locked = CoreLocal::get().scheduler_input.lock();

		while let Some(task) = input_locked.wakeup_tasks.pop_front() {
			let task = self.blocked_tasks.custom_wakeup(task);
			self.ready_queue.push(task);
		}

		while let Some(new_task) = input_locked.new_tasks.pop_front() {
			let task = Rc::new(RefCell::new(Task::from(new_task)));
			self.ready_queue.push(task.clone());
		}
	}

	/// Only the idle task should call this function.
	/// Set the idle task to halt state if not another
	/// available.
	pub fn run() -> ! {
		let backoff = Backoff::new();

		loop {
			let core_scheduler = core_scheduler();
			Arch::disable();

			// do housekeeping
			core_scheduler.check_input();
			core_scheduler.cleanup_tasks();

			if core_scheduler.ready_queue.is_empty() {
				if backoff.is_completed() {
					Arch::enable_and_wait();
					backoff.reset();
				} else {
					Arch::enable();
					backoff.snooze();
				}
			} else {
				Arch::enable();
				core_scheduler.reschedule();
				backoff.reset();
			}
		}
	}

	#[inline]
	#[cfg(target_arch = "aarch64")]
	pub fn get_last_stack_pointer(&self) -> memory_addresses::VirtAddr {
		self.current_task.borrow().last_stack_pointer
	}

	/// Triggers the scheduler to reschedule the tasks.
	/// Interrupt flag must be cleared before calling this function.
	pub fn scheduler(&mut self) -> Option<*mut usize> {
		// Someone wants to give up the CPU
		// => we have time to cleanup the system
		self.cleanup_tasks();

		// Get information about the current task.
		let (id, last_stack_pointer, prio, status) = {
			let mut borrowed = self.current_task.borrow_mut();
			(
				borrowed.id,
				ptr::from_mut(&mut borrowed.last_stack_pointer).cast::<usize>(),
				borrowed.prio,
				borrowed.status,
			)
		};

		let mut new_task = None;

		if status == TaskStatus::Running {
			// A task is currently running.
			// Check if a task with a equal or higher priority is available.
			if let Some(task) = self.ready_queue.pop_with_prio(prio) {
				new_task = Some(task);
			}
		} else {
			if status == TaskStatus::Finished {
				// Mark the finished task as invalid and add it to the finished tasks for a later cleanup.
				self.current_task.borrow_mut().status = TaskStatus::Invalid;
				self.finished_tasks.push_back(self.current_task.clone());
			}

			// No task is currently running.
			// Check if there is any available task and get the one with the highest priority.
			if let Some(task) = self.ready_queue.pop() {
				// This available task becomes the new task.
				debug!("Task is available.");
				new_task = Some(task);
			} else if status != TaskStatus::Idle {
				// The Idle task becomes the new task.
				debug!("Only Idle Task is available.");
				new_task = Some(self.idle_task.clone());
			}
		}

		if let Some(task) = new_task {
			// There is a new task we want to switch to.

			// Handle the current task.
			if status == TaskStatus::Running {
				// Mark the running task as ready again and add it back to the queue.
				self.current_task.borrow_mut().status = TaskStatus::Ready;
				self.ready_queue.push(self.current_task.clone());
			}

			// Handle the new task and get information about it.
			let (new_id, new_stack_pointer) = {
				let mut borrowed = task.borrow_mut();
				if borrowed.status != TaskStatus::Idle {
					// Mark the new task as running.
					borrowed.status = TaskStatus::Running;
				}

				(borrowed.id, borrowed.last_stack_pointer)
			};

			if id != new_id {
				// Tell the scheduler about the new task.
				debug!(
					"Switching task from {} to {} (stack {:#X} => {:p})",
					id,
					new_id,
					unsafe { *last_stack_pointer },
					new_stack_pointer
				);
				#[cfg(not(target_arch = "riscv64"))]
				{
					self.current_task = task;
				}

				// Finally return the context of the new task.
				#[cfg(not(target_arch = "riscv64"))]
				return Some(last_stack_pointer);

				#[cfg(target_arch = "riscv64")]
				{
					if sstatus::read().fs() == sstatus::FS::Dirty {
						self.current_task.borrow_mut().last_fpu_state.save();
					}
					task.borrow().last_fpu_state.restore();
					self.current_task = task;
					unsafe {
						switch_to_task(last_stack_pointer, new_stack_pointer.as_usize());
					}
				}
			}
		}

		None
	}
}

fn get_tid() -> TaskId {
	static TID_COUNTER: AtomicI32 = AtomicI32::new(0);
	let guard = TASKS.lock();

	loop {
		let id = TaskId::from(TID_COUNTER.fetch_add(1, Ordering::SeqCst));
		if !guard.contains_key(&id) {
			return id;
		}
	}
}

/// Add a per-core scheduler for the current core.
pub(crate) fn add_current_core() {
	// Create an idle task for this core.
	let core_id = core_id();
	let tid = get_tid();
	let idle_task = Rc::new(RefCell::new(Task::new_idle(tid, core_id)));

	// Add the ID -> Task mapping.
	WAITING_TASKS.lock().insert(tid, VecDeque::with_capacity(1));
	TASKS
		.lock()
		.insert(tid, TaskHandle::new(tid, IDLE_PRIO, core_id));
	// Initialize a scheduler for this core.
	debug!("Initializing scheduler for core {core_id} with idle task {tid}");
	let boxed_scheduler = Box::new(PerCoreScheduler {
		core_id,
		current_task: idle_task.clone(),
		#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
		fpu_owner: idle_task.clone(),
		idle_task,
		ready_queue: PriorityTaskQueue::new(),
		finished_tasks: VecDeque::new(),
		blocked_tasks: BlockedTaskQueue::new(),
	});

	let scheduler = Box::into_raw(boxed_scheduler);
	set_core_scheduler(scheduler);
	{
		SCHEDULER_INPUTS.lock().insert(
			core_id.try_into().unwrap(),
			&CoreLocal::get().scheduler_input,
		);
		#[cfg(target_arch = "x86_64")]
		CORE_HLT_STATE
			.lock()
			.insert(core_id.try_into().unwrap(), &CoreLocal::get().hlt);
	}
}

#[inline]
#[cfg(all(target_arch = "x86_64", not(feature = "idle-poll")))]
pub(crate) fn take_core_hlt_state(core_id: CoreId) -> bool {
	CORE_HLT_STATE.lock()[usize::try_from(core_id).unwrap()].swap(false, Ordering::Acquire)
}

#[inline]
fn get_scheduler_input(core_id: CoreId) -> &'static InterruptTicketMutex<SchedulerInput> {
	SCHEDULER_INPUTS.lock()[usize::try_from(core_id).unwrap()]
}

pub unsafe fn spawn(
	func: unsafe extern "C" fn(usize),
	arg: usize,
	prio: Priority,
	stack_size: usize,
	selector: isize,
) -> TaskId {
	static CORE_COUNTER: AtomicU32 = AtomicU32::new(1);

	let core_id = if selector < 0 {
		// use Round Robin to schedule the cores
		CORE_COUNTER.fetch_add(1, Ordering::SeqCst) % get_processor_count()
	} else {
		selector as u32
	};

	unsafe { PerCoreScheduler::spawn(func, arg, prio, core_id, stack_size) }
}

#[allow(clippy::result_unit_err)]
pub fn join(id: TaskId) -> Result<(), ()> {
	let core_scheduler = core_scheduler();

	debug!(
		"Task {} is waiting for task {}",
		core_scheduler.get_current_task_id(),
		id
	);

	loop {
		let mut waiting_tasks_guard = WAITING_TASKS.lock();

		if let Some(queue) = waiting_tasks_guard.get_mut(&id) {
			queue.push_back(core_scheduler.get_current_task_handle());
			core_scheduler.block_current_task(None);

			// Switch to the next task.
			drop(waiting_tasks_guard);
			core_scheduler.reschedule();
		} else {
			return Ok(());
		}
	}
}
