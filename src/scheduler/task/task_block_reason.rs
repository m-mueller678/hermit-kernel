use crate::scheduler::task::TaskId;

#[derive(Debug)]
pub enum TaskBlockReason {
	Join(TaskId),
	Futex(usize),
	Semaphore(usize),
	Usleep(u64),
	ExplicitBlockSyscall,
}

const NOT_BLOCKED: u64 = 0;
const JOIN: u64 = 1;
const FUTEX: u64 = 2;
const SEMAPHORE: u64 = 3;
const USLEEP: u64 = 4;
const EXPLICIT_BLOCK_SYSCALL: u64 = 5;
const TAG_END: u64 = 6;

const TAG_BITS: u32 = 3;
const PAYLOAD_BITS: u32 = 64 - TAG_BITS;

impl TaskBlockReason {
	pub fn encode(this: Option<Self>) -> u64 {
		let Some(this) = this else {
			return u64_from_parts(NOT_BLOCKED, 0);
		};
		match this {
			TaskBlockReason::Join(task) => u64_from_parts(JOIN, task.into() as u64),
			TaskBlockReason::Futex(address) => u64_from_parts(FUTEX, address_to_payload(address)),
			TaskBlockReason::Semaphore(address) => {
				u64_from_parts(SEMAPHORE, address_to_payload(address))
			}
			TaskBlockReason::Usleep(duration) => {
				u64_from_parts(USLEEP, duration.min((1 << PAYLOAD_BITS) - 1))
			}
			TaskBlockReason::ExplicitBlockSyscall => u64_from_parts(EXPLICIT_BLOCK_SYSCALL, 0),
		}
	}

	pub fn decode(encoded: u64) -> Option<Self> {
		let tag = encoded & ((1 << TAG_BITS) - 1);
		let payload = encoded >> TAG_BITS;
		match tag {
			NOT_BLOCKED => None,
			JOIN => TaskBlockReason::Join(TaskId::from(payload)),
			FUTEX => TaskBlockReason::Futex(payload_to_address(payload)),
			SEMAPHORE => TaskBlockReason::Semaphore(payload_to_address(payload)),
			USLEEP => TaskBlockReason::Usleep(payload),
		}
	}
}

fn u64_from_parts(tag: u64, payload: u64) -> u64 {
	assert!(tag < (1 << PAYLOAD_BITS));
	assert!(tag < (1 << TAG_BITS));
	payload << TAG_BITS + tag
}

fn address_to_payload(address: usize) -> u64 {
	let ret = address & (u64::MAX >> TAG_BITS);
	assert!(payload_to_address(ret) == addres);
	ret
}

fn payload_to_address(payload: u64) -> usize {
	(payload as i64) << TAG_BITS >> TAG_BITS as usize
}
