use super::Tracker;
use crate::channel::tracker::ReceiverTracker;
use crate::channel::wait_strategy::WaitStrategy;
use crate::channel::FastMod;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicIsize, AtomicUsize, Ordering};

#[derive(Debug)]
pub struct MultiCursorTracker<WS> {
    // Access will always be write so no need for a more complex read write lock here.
    // It shouldn't be accessed too much and should only impede new/dying receivers not active
    // senders or receivers
    counters: Vec<AtomicUsize>,
    num_receivers: AtomicUsize,
    wait_strategy: WS,
}

impl<WS> MultiCursorTracker<WS>
where
    WS: WaitStrategy,
{
    pub fn new(mut size: usize, wait_strategy: WS) -> Self {
        let size = (size + 1).next_power_of_two();
        let mut counters = Vec::new();
        counters.resize_with(size, Default::default);
        Self {
            counters,
            num_receivers: Default::default(),
            wait_strategy,
        }
    }
}

impl<WS> ReceiverTracker for MultiCursorTracker<WS>
where
    WS: WaitStrategy,
{
    fn register(&self) -> isize {
        self.num_receivers.fetch_add(1, Ordering::SeqCst);
        self.update(-1, 0);
        0
    }

    fn update(&self, from: isize, to: isize) {
        if to < 0 {
            panic!("broadcast tracker only works with positive values")
        }
        if to == from {
            return;
        }
        let to_idx = (to as usize).fmod(self.counters.len());
        let from_idx = (from as usize).fmod(self.counters.len());

        unsafe {
            self.counters
                .get_unchecked(to_idx)
                .fetch_add(1, Ordering::SeqCst);
            if from >= 0 {
                self.counters
                    .get_unchecked(from_idx)
                    .fetch_sub(1, Ordering::SeqCst);
            }
        }
        self.wait_strategy.notify();
    }

    fn de_register(&self, at: isize) {
        if at >= 0 {
            let index = (at as usize).fmod(self.counters.len());
            let previous;
            unsafe {
                previous = self
                    .counters
                    .get_unchecked(index)
                    .fetch_sub(1, Ordering::SeqCst);
            }
            debug_assert!(previous > 0);
        }
        self.num_receivers.fetch_sub(1, Ordering::SeqCst);
    }
}
impl<WS> Tracker for MultiCursorTracker<WS>
where
    WS: WaitStrategy,
{
    fn wait_for(&self, expected_tail: isize) -> isize {
        let index = (expected_tail as usize).fmod(self.counters.len());
        unsafe {
            self.wait_strategy
                .wait_for_eq(self.counters.get_unchecked(index), 0);
        }
        expected_tail
    }

    fn current(&self) -> isize {
        -1
    }
}

#[cfg(test)]
mod tracker_tests {
    use super::*;
    use crate::channel::wait_strategy::BusyWait;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn add_remove_receiver() {
        let tracker = MultiCursorTracker::new(10, BusyWait::default());
        let shared_cursor_a = tracker.register();
        assert_eq!(tracker.counters[0].load(Ordering::Acquire), 1);
        tracker.update(shared_cursor_a, 4);
        assert_eq!(tracker.counters[0].load(Ordering::Acquire), 0);
        assert_eq!(tracker.counters[4].load(Ordering::Acquire), 1);
        assert_eq!(tracker.current(), 4);

        let shared_cursor_b = tracker.register();
        tracker.update(shared_cursor_b, 5);
        assert_eq!(tracker.counters[5].load(Ordering::Acquire), 1);
        assert_eq!(tracker.current(), 4);

        tracker.de_register(4);
        assert_eq!(tracker.counters[4].load(Ordering::Acquire), 0);
        assert_eq!(tracker.current(), 5);

        tracker.update(5, 7);
        assert_eq!(tracker.counters[5].load(Ordering::Acquire), 0);
        assert_eq!(tracker.counters[6].load(Ordering::Acquire), 0);
        assert_eq!(tracker.counters[7].load(Ordering::Acquire), 1);
        assert_eq!(tracker.current(), 7);

        tracker.de_register(7);
    }

    #[test]
    fn wait_for_tail() {
        let tracker = MultiCursorTracker::new(10, BusyWait::default());
        let shared_cursor_a = tracker.register();
        tracker.update(shared_cursor_a, 4);

        thread::scope(|s| {
            let th = s.spawn(|| {
                assert_eq!(tracker.current(), 4);
                tracker.wait_for(9);
                assert_eq!(tracker.current(), 9);
            });
            thread::sleep(Duration::from_millis(50));
            for i in 5..10 {
                tracker.update(i - 1, i);
                thread::sleep(Duration::from_millis(10));
            }
            th.join().expect("couldn't join");
        });
    }
}
