#[cfg(feature = "alloc-stats")]
mod alloc_stats;
pub(crate) mod task;

use core::future::Future;
use core::task::Waker;

use hermit_sync::without_interrupts;

use crate::arch::core_local;
use crate::executor::task::AsyncTask;

/// WakerRegistration is derived from smoltcp's
/// implementation.
#[derive(Debug)]
pub(crate) struct WakerRegistration {
	waker: Option<Waker>,
}

impl WakerRegistration {
	pub const fn new() -> Self {
		Self { waker: None }
	}

	/// Wake the registered waker, if any.
	#[allow(dead_code)]
	pub fn wake(&mut self) {
		if let Some(w) = self.waker.take() {
			w.wake();
		}
	}
}

pub(crate) fn run() {
	without_interrupts(|| {
		// FIXME: We currently have no more than 3 tasks at a time, so this is fine.
		// Ideally, we would set this value to 200, but the network task currently immediately wakes up again.
		// This would lead to the network task being polled 200 times back to back, slowing things down considerably.
		for _ in 0..3 {
			if !core_local::ex().try_tick() {
				break;
			}
		}
	});
}

/// Spawns a future on the executor.
#[cfg_attr(not(any(feature = "alloc-stats")), expect(dead_code))]
pub(crate) fn spawn<F>(future: F)
where
	F: Future<Output = ()> + Send + 'static,
{
	core_local::ex().spawn(AsyncTask::new(future)).detach();
}

pub fn init() {
	#[cfg(feature = "alloc-stats")]
	crate::executor::alloc_stats::init();
}
