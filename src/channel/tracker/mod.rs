pub mod broadcast_tracker;

use crate::channel::wait_strategy::WaitStrategy;
use core::sync::atomic::{AtomicIsize, Ordering};

pub trait ReceiverTracker {
    fn register(&self, at: isize) -> isize;
    fn update(&self, from: isize, to: isize);
    fn de_register(&self, at: isize);
}

pub trait ProducerTracker {
    fn make_claim(&self) -> isize;
    fn commit_claim(&self, id: isize);
    fn publish(&self, id: isize);
    fn current(&self) -> isize;
}

pub trait Tracker {
    fn wait_for(&self, expected: isize) -> isize;
}

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
