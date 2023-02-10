use super::Tracker;
use crate::channel::tracker::ReceiverTracker;
use crate::channel::wait_strategy::WaitStrategy;
use crate::channel::FastMod;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicUsize, Ordering};
use std::sync::atomic::AtomicIsize;

#[derive(Debug)]
pub struct MultiCursorTracker<WS> {
    // Access will always be write so no need for a more complex read write lock here.
    // It shouldn't be accessed too much and should only impede new/dying receivers not active
    // senders or receivers
    counters: Vec<AtomicUsize>,
    tail: AtomicIsize,
    wait_strategy: WS,
}

impl<WS> MultiCursorTracker<WS>
where
    WS: WaitStrategy,
{
    pub fn new(mut size: usize, wait_strategy: WS) -> Self {
        // This is very inefficient but it's to prevent collision on wrapping
        size = (size + 1).next_power_of_two();
        let mut counters = Vec::new();
        counters.resize_with(size, Default::default);
        Self {
            counters,
            tail: Default::default(),
            wait_strategy,
        }
    }
}

impl<WS> ReceiverTracker for MultiCursorTracker<WS>
where
    WS: WaitStrategy,
{
    fn register(&self, mut at: isize) -> isize {
        //TODO there could be a race here if it gets overwritten
        at = at.clamp(0, isize::MAX);
        let idx = (at as usize).pow_2_mod(self.counters.len());
        unsafe {
            self.counters
                .get_unchecked(idx)
                .fetch_add(1, Ordering::SeqCst);
        }
        at
    }

    fn update(&self, from: isize, to: isize) {
        debug_assert!(to >= 0);
        debug_assert!(from >= 0);
        debug_assert!(from < to);

        let to_idx = (to as usize).pow_2_mod(self.counters.len());
        let from_idx = (from as usize).pow_2_mod(self.counters.len());

        let previous;
        unsafe {
            self.counters
                .get_unchecked(to_idx)
                .fetch_add(1, Ordering::SeqCst);
            previous = self
                .counters
                .get_unchecked(from_idx)
                .fetch_sub(1, Ordering::SeqCst);
        }
        if previous == 1 && self.tail.load(Ordering::Acquire) == from {
            self.tail.store(to, Ordering::Release);
            //the tail has moved. notify anyone who was listening
            self.wait_strategy.notify();
        }
    }

    fn de_register(&self, at: isize) {
        if at >= 0 {
            let index = (at as usize).pow_2_mod(self.counters.len());
            let previous;
            unsafe {
                previous = self
                    .counters
                    .get_unchecked(index)
                    .fetch_sub(1, Ordering::SeqCst);
            }
            debug_assert!(previous > 0);
        }
    }
}
impl<WS> Tracker for MultiCursorTracker<WS>
where
    WS: WaitStrategy,
{
    fn wait_for(&self, expected_tail: isize) -> isize {
        self.wait_strategy.wait_for_geq(&self.tail, expected_tail)
    }

    fn current(&self) -> isize {
        self.tail.load(Ordering::Acquire)
    }
}

#[cfg(test)]
mod tracker_tests {
    use super::*;
    use crate::channel::wait_strategy::BusyWait;

    #[test]
    fn add_remove_receiver() {
        let tracker = MultiCursorTracker::new(10, BusyWait::default());
        let shared_cursor_a = tracker.register(0);
        assert_eq!(tracker.counters[0].load(Ordering::Acquire), 1);
        tracker.update(shared_cursor_a, 4);
        assert_eq!(tracker.counters[0].load(Ordering::Acquire), 0);
        assert_eq!(tracker.counters[4].load(Ordering::Acquire), 1);

        let shared_cursor_b = tracker.register(2);
        assert_eq!(tracker.counters[2].load(Ordering::Acquire), 1);
        tracker.update(shared_cursor_b, 5);
        assert_eq!(tracker.counters[5].load(Ordering::Acquire), 1);

        tracker.de_register(4);
        assert_eq!(tracker.counters[4].load(Ordering::Acquire), 0);

        tracker.update(5, 7);
        assert_eq!(tracker.counters[5].load(Ordering::Acquire), 0);
        assert_eq!(tracker.counters[6].load(Ordering::Acquire), 0);
        assert_eq!(tracker.counters[7].load(Ordering::Acquire), 1);

        tracker.de_register(7);
    }
}
