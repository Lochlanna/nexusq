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
            if self.num_receivers.load(Ordering::Acquire) < 1 {
                //leave the tail as is :(
                return;
            }
            let index = (id as usize).fmod(self.counters.len());
            let cell;
            unsafe {
                cell = self.counters.get_unchecked(index);
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
        self.num_receivers.fetch_add(1, Ordering::Relaxed);
        let tail_pos = self.tail.load(Ordering::Relaxed);
        let index = (tail_pos as usize).fmod(self.counters.len());
        //TODO there is a race condition here! The tail could move!
        unsafe {
            let cell = self.counters.get_unchecked(index);
            cell.fetch_add(1, Ordering::Relaxed);
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

        if previous == 1
            && self
                .tail
                .compare_exchange(from, to, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
        {
            // we have moved the tail!
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
        let tail = self.tail.load(Ordering::Acquire);
        if previous == 1 && at == tail {
            if self.num_receivers.fetch_sub(1, Ordering::SeqCst) > 1 {
                self.chase_tail(at);
            }
            // we have just removed the tail receiver so we need to chase the tail to find it
        } else {
            self.num_receivers.fetch_sub(1, Ordering::SeqCst);
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
