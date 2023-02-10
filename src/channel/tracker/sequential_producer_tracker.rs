use super::{ProducerTracker, Tracker};
use crate::WaitStrategy;
use std::sync::atomic::{AtomicIsize, Ordering};

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
}

impl<WS> ProducerTracker for SequentialProducerTracker<WS>
where
    WS: WaitStrategy,
{
    fn make_claim(&self) -> isize {
        let claim = self.claimed.fetch_add(1, Ordering::Acquire);
        let expected = claim - 1;
        while self.committed.load(Ordering::Relaxed) != expected {
            core::hint::spin_loop();
        }
        claim
    }

    fn commit_claim(&self, id: isize) {
        self.committed.store(id, Ordering::Release);
    }

    fn publish(&self, id: isize) {
        let expected = id - 1;
        while self
            .published
            .compare_exchange_weak(expected, id, Ordering::Relaxed, Ordering::Relaxed)
            .is_err()
        {
            core::hint::spin_loop();
        }
        self.wait_strategy.notify();
    }

    fn current(&self) -> isize {
        self.published.load(Ordering::Acquire)
    }
}
