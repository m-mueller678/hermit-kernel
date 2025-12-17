use ahash::RandomState;
use hashbrown::HashMap;

use crate::drivers::{InterruptHandlerQueue, InterruptLine};

pub(crate) fn get_interrupt_handlers() -> HashMap<InterruptLine, InterruptHandlerQueue, RandomState>
{
	HashMap::with_hasher(RandomState::with_seeds(0, 0, 0, 0))
}
