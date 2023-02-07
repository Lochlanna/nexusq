use super::Tracker;
use crate::channel::tracker::ReceiverTracker;
use crate::channel::wait_strategy::{WaitStrategy, Waitable};
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
    num_receivers: AtomicUsize,
    tail: AtomicIsize,
    wait_strategy: WS,
}

impl<WS> MultiCursorTracker<WS>
where
    WS: WaitStrategy,
{
    fn chase_tail(&self, from: isize) {
        let mut id = from;
        loop {
            //TODO maybe there is an optimisation here...
            let cell;
            unsafe {
                cell = self
                    .counters
                    .get_unchecked((id as usize).fmod(self.counters.len()));
            }
            if cell.load(Ordering::Acquire) > 0 {
                if cell.fetch_add(1, Ordering::SeqCst) > 0 {
                    self.tail.store(id, Ordering::Release);
                    if cell.fetch_sub(1, Ordering::SeqCst) > 1 {
                        // the reader was still on this cell we have successfully set the tail
                        break;
                    }
                    //keep looping
                } else {
                    // the receiver moved off between load and fetch_add so sub here
                    cell.fetch_sub(1, Ordering::SeqCst);
                }
            }
            id += 1;
        }
        self.wait_strategy.notify();
    }
    pub fn new(mut size: usize, wait_strategy: WS) -> Self {
        //TODO this could get really massive...
        size = (size + 1)
            .checked_next_power_of_two()
            .expect("usize wrapped!");
        let mut counters = Vec::new();
        counters.resize_with(size, Default::default);
        Self {
            counters,
            num_receivers: Default::default(),
            tail: Default::default(),
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
        let tail_pos = self.tail.load(Ordering::Acquire);
        let index = (tail_pos as usize).fmod(self.counters.len());
        //TODO there is a race condition here!
        unsafe {
            self.counters
                .get_unchecked(index)
                .fetch_add(1, Ordering::SeqCst);
        }
        tail_pos
    }

    fn update(&self, from: isize, to: isize) {
        if from < 0 || to < 0 {
            panic!("broadcast tracker only works with positive values")
        }
        if to == from {
            return;
        }
        let from_idx = (from as usize).fmod(self.counters.len());
        let to_idx = (to as usize).fmod(self.counters.len());
        let to_cell;
        let from_cell;

        unsafe {
            to_cell = self.counters.get_unchecked(to_idx);
            from_cell = self.counters.get_unchecked(from_idx);
        }

        to_cell.fetch_add(1, Ordering::SeqCst);
        let previous = from_cell.fetch_sub(1, Ordering::SeqCst);

        //Assume there is at least one receiver so don't need to check here
        if previous == 1
            && self
                .tail
                .compare_exchange(from, to, Ordering::SeqCst, Ordering::Acquire)
                .is_ok()
        {
            self.wait_strategy.notify();
        }
    }

    fn de_register(&self, at: isize) {
        let index = (at as usize).fmod(self.counters.len());
        let previous;
        unsafe {
            previous = self
                .counters
                .get_unchecked(index)
                .fetch_sub(1, Ordering::SeqCst);
        }
        debug_assert!(previous > 0);
        let num_left = self.num_receivers.fetch_sub(1, Ordering::SeqCst) - 1;
        //TODO what to do with the tail when we go to zero...??? currently will lock the sender forever!!
        // We could return error on slowest to notify sender of this
        if previous == 1 && at == self.tail.load(Ordering::Acquire) && num_left > 0 {
            // we have just removed the tail receiver so we need to chase the tail to find it
            // self.chase_tail(at);
        }
    }
}
impl<WS> Tracker for MultiCursorTracker<WS>
where
    WS: WaitStrategy,
{
    fn wait_for(&self, expected_tail: isize) -> isize {
        self.wait_strategy.wait_for(&self.tail, expected_tail)
    }

    fn current(&self) -> isize {
        self.tail.load(Ordering::Acquire)
    }
}

#[cfg(test)]
mod tracker_tests {
    use super::*;
    use crate::channel::wait_strategy::BusySpinWaitStrategy;
    use std::thread;
    use std::thread::current;
    use std::time::Duration;

    // #[test]
    // fn add_remove_receiver() {
    //     let tracker = BroadcastTracker::new(10);
    //     let shared_cursor_a = tracker.new_receiver();
    //     assert_eq!(tracker.counters[0].load(Ordering::SeqCst), 1);
    //     tracker.move_receiver(shared_cursor_a, 4);
    //     assert_eq!(tracker.counters[0].load(Ordering::SeqCst), 0);
    //     assert_eq!(tracker.counters[4].load(Ordering::SeqCst), 1);
    //     assert_eq!(tracker.slowest(), 4);
    //
    //     let shared_cursor_b = tracker.new_receiver();
    //     tracker.move_receiver(shared_cursor_b, 6);
    //     assert_eq!(tracker.counters[6].load(Ordering::SeqCst), 1);
    //     assert_eq!(tracker.slowest(), 4);
    //
    //     tracker.remove_receiver(4);
    //     assert_eq!(tracker.counters[4].load(Ordering::SeqCst), 0);
    //     assert_eq!(tracker.slowest(), 6);
    //
    //     tracker.move_receiver(6, 7);
    //     assert_eq!(tracker.counters[6].load(Ordering::SeqCst), 0);
    //     assert_eq!(tracker.counters[7].load(Ordering::SeqCst), 1);
    //     assert_eq!(tracker.slowest(), 7);
    // }

    #[test]
    fn wait_for_tail() {
        let tracker = MultiCursorTracker::new(10, BusySpinWaitStrategy::default());
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
