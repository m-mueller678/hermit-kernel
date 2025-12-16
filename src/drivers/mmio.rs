#[cfg(feature = "console")]
use alloc::collections::VecDeque;

use ahash::RandomState;
use hashbrown::HashMap;

#[cfg(feature = "console")]
pub(crate) use crate::arch::kernel::mmio::get_console_driver;
use crate::drivers::{InterruptHandlerQueue, InterruptLine};

pub(crate) fn get_interrupt_handlers() -> HashMap<InterruptLine, InterruptHandlerQueue, RandomState>
{
	#[allow(unused_mut)]
	let mut handlers: HashMap<InterruptLine, InterruptHandlerQueue, RandomState> =
		HashMap::with_hasher(RandomState::with_seeds(0, 0, 0, 0));

	#[cfg(feature = "console")]
	if let Some(drv) = get_console_driver() {
		fn console_handler() {
			if let Some(driver) = get_console_driver() {
				driver.lock().handle_interrupt();
			}
		}

		let irq_number = drv.lock().get_interrupt_number();

		if let Some(map) = handlers.get_mut(&irq_number) {
			map.push_back(console_handler);
		} else {
			let mut map: InterruptHandlerQueue = VecDeque::new();
			map.push_back(console_handler);
			handlers.insert(irq_number, map);
		}
	}

	handlers
}
