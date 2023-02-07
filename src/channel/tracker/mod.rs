pub mod broadcast_tracker;

use crate::channel::wait_strategy::WaitStrategy;
use std::sync::atomic::{AtomicIsize, Ordering};

pub trait ReceiverTracker {
    fn register(&self) -> isize;
    fn update(&self, from: isize, to: isize);
    fn de_register(&self, at: isize);
}

pub trait ProducerTracker {
    fn claim(&self, num_to_claim: usize) -> isize;
    fn commit(&self, id: isize);
}

pub trait Tracker {
    fn wait_for(&self, expected: isize) -> isize;
    fn current(&self) -> isize;
}

#[derive(Debug)]
pub struct SequentialProducerTracker<WS> {
    claimed: AtomicIsize,
    committed: AtomicIsize,
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
            wait_strategy,
        }
    }
}

impl<WS> Tracker for SequentialProducerTracker<WS>
where
    WS: WaitStrategy,
{
    fn wait_for(&self, expected: isize) -> isize {
        self.wait_strategy.wait_for(&self.committed, expected)
    }

    fn current(&self) -> isize {
        self.committed.load(Ordering::Acquire)
    }
}

impl<WS> ProducerTracker for SequentialProducerTracker<WS>
where
    WS: WaitStrategy,
{
    fn claim(&self, num_to_claim: usize) -> isize {
        self.claimed
            .fetch_add(num_to_claim as isize, Ordering::SeqCst)
    }

    fn commit(&self, id: isize) {
        let expected = id - 1;
        self.wait_strategy.wait_for(&self.committed, expected);
        debug_assert!(self.committed.load(Ordering::Acquire) == expected);
        self.committed.store(id, Ordering::Release);
        self.wait_strategy.notify();
    }
}
