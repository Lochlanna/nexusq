use super::{ProducerTracker, Tracker};
use crate::WaitStrategy;
use std::sync::atomic::{AtomicIsize, AtomicUsize, Ordering};

#[derive(Debug)]
pub struct SequentialProducerTracker<WS> {
    claimed: AtomicIsize,
    committed: AtomicIsize,
    published: AtomicIsize,
    wait_strategy: WS,
}

impl<WS> SequentialProducerTracker<WS>
where
    WS: WaitStrategy,
{
    pub fn new(wait_strategy: WS) -> Self {
        Self {
            claimed: Default::default(),
            committed: AtomicIsize::new(-1),
            published: AtomicIsize::new(-1),
            wait_strategy,
        }
    }
}

impl<WS> Tracker for SequentialProducerTracker<WS>
where
    WS: WaitStrategy,
{
    fn wait_for(&self, expected: isize) -> isize {
        self.wait_strategy.wait_for_geq(&self.published, expected)
    }

    fn current(&self) -> isize {
        self.published.load(Ordering::Acquire)
    }
}

impl<WS> ProducerTracker for SequentialProducerTracker<WS>
where
    WS: WaitStrategy,
{
    fn make_claim(&self) -> isize {
        self.claimed.fetch_add(1, Ordering::SeqCst)
    }

    fn publish(&self, id: isize) {
        while self.published.load(Ordering::Acquire) != id - 1 {
            core::hint::spin_loop();
        }
        self.published.store(id, Ordering::Release);
        self.wait_strategy.notify();
    }
}
