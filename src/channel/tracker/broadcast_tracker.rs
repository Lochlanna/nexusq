use super::Tracker;
use crate::channel::tracker::ReceiverTracker;
use crate::channel::wait_strategy::WaitStrategy;
use crate::channel::{tracker, FastMod};
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
    num_readers: AtomicIsize,
}

impl<WS> MultiCursorTracker<WS>
where
    WS: WaitStrategy,
{
    pub fn new(size: usize, wait_strategy: WS) -> Result<Self, super::TrackerError> {
        // This is very inefficient but it's to prevent collision on wrapping
        if !size.is_power_of_two() {
            return Err(super::TrackerError::InvalidSize);
        }
        let mut counters = Vec::new();
        counters.resize_with(size, Default::default);
        Ok(Self {
            counters,
            tail: Default::default(),
            wait_strategy,
            num_readers: Default::default(),
        })
    }

    fn chase_tail(&self, from: isize) {
        //find the next tail by iterating over the ring
        let mut current_id = from as usize;
        loop {
            if self.num_readers.load(Ordering::Acquire) == 0 {
                // There are no readers left!
                break;
            }
            let index = current_id.pow_2_mod(self.counters.len());
            let cell;
            unsafe {
                cell = self.counters.get_unchecked(index);
            }
            if cell.load(Ordering::Acquire) > 0 {
                self.tail.store(current_id as isize, Ordering::Release);
                if cell.load(Ordering::Acquire) != 0
                    || self.tail.load(Ordering::Acquire) > (current_id as isize)
                {
                    return;
                }
                debug_assert!(self.tail.load(Ordering::Acquire) == current_id as isize);
            }
            current_id += 1;
        }
    }
}

impl<WS> ReceiverTracker for MultiCursorTracker<WS>
where
    WS: WaitStrategy,
{
    fn register(&self, mut at: isize) -> Result<isize, tracker::TrackerError> {
        at = at.clamp(0, isize::MAX);
        if at < self.tail.load(Ordering::Acquire) {
            return Err(tracker::TrackerError::PositionTooOld);
        }
        let idx = (at as usize).pow_2_mod(self.counters.len());
        unsafe {
            self.counters
                .get_unchecked(idx)
                .fetch_add(1, Ordering::SeqCst);
        }
        if at < self.tail.load(Ordering::Acquire) {
            // we missed it. Undo
            unsafe {
                let previous = self
                    .counters
                    .get_unchecked(idx)
                    .fetch_sub(1, Ordering::SeqCst);
                debug_assert!(previous == 1);
            }
            return Err(tracker::TrackerError::PositionTooOld);
        }
        self.num_readers.fetch_add(1, Ordering::Release);
        Ok(at)
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
            let num_readers_left = self.num_readers.fetch_sub(1, Ordering::Release) - 1;
            let index = (at as usize).pow_2_mod(self.counters.len());
            let previous;
            unsafe {
                previous = self
                    .counters
                    .get_unchecked(index)
                    .fetch_sub(1, Ordering::SeqCst);
            }
            if num_readers_left == 0 {
                return;
            }
            if previous == 1 && at == self.tail.load(Ordering::Acquire) {
                self.chase_tail(at);
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
    fn invalid_size() {
        let create = |s: usize| MultiCursorTracker::new(s, BusyWait::default());
        for size in 0..300 {
            let res = create(size);
            if size.is_power_of_two() {
                assert!(res.is_ok());
            } else {
                assert!(res.is_err());
            }
        }
    }

    #[test]
    fn add_remove_receiver() {
        let tracker = MultiCursorTracker::new(16, BusyWait::default())
            .expect("couldn't create multi cursor tracker");
        let shared_cursor_a = tracker.register(0).expect("couldn't register");
        assert_eq!(tracker.counters[0].load(Ordering::Acquire), 1);
        tracker.update(shared_cursor_a, 4);
        assert_eq!(tracker.counters[0].load(Ordering::Acquire), 0);
        assert_eq!(tracker.counters[4].load(Ordering::Acquire), 1);

        assert!(tracker.register(2).is_err());

        let shared_cursor_b = tracker.register(4).expect("couldn't register");
        assert_eq!(tracker.counters[4].load(Ordering::Acquire), 2);
        tracker.update(shared_cursor_b, 6);
        assert_eq!(tracker.counters[6].load(Ordering::Acquire), 1);
        assert_eq!(tracker.counters[4].load(Ordering::Acquire), 1);

        tracker.de_register(4);
        assert_eq!(tracker.counters[4].load(Ordering::Acquire), 0);
        assert_eq!(tracker.tail.load(Ordering::Acquire), 6);

        tracker.update(6, 7);
        assert_eq!(tracker.counters[6].load(Ordering::Acquire), 0);
        assert_eq!(tracker.counters[7].load(Ordering::Acquire), 1);
        assert_eq!(tracker.tail.load(Ordering::Acquire), 7);

        tracker.de_register(7);
        assert_eq!(tracker.tail.load(Ordering::Acquire), 7);
        assert_eq!(tracker.num_readers.load(Ordering::Acquire), 0);
    }
}
