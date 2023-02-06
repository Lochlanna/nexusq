use super::Tracker;
use alloc::boxed::Box;
use alloc::vec::Vec;
use async_trait::async_trait;
use core::sync::atomic::{AtomicUsize, Ordering};

#[derive(Debug)]
pub struct BroadcastTracker {
    // Access will always be write so no need for a more complex read write lock here.
    // It shouldn't be accessed too much and should only impede new/dying receivers not active
    // senders or receivers
    counters: Vec<AtomicUsize>,
    num_receivers: AtomicUsize,
    tail: AtomicUsize,
    tail_move: event_listener::Event,
}

impl BroadcastTracker {
    fn to_index(&self, v: isize) -> usize {
        if v < 0 {
            return 0;
        }
        v as usize % self.counters.len()
    }

    fn chase_tail(&self, from: usize) {
        let mut id = from;
        loop {
            //TODO maybe there is an optimisation here...
            let cell;
            unsafe {
                cell = self.counters.get_unchecked(id % self.counters.len());
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
        self.tail_move.notify(usize::MAX);
    }

    fn wait_for_shared(
        &self,
        expected_tail: usize,
    ) -> Result<usize, event_listener::EventListener> {
        let mut tail = self.tail.load(Ordering::Acquire);
        if tail >= expected_tail {
            return Ok(tail);
        }
        let listener = self.tail_move.listen();
        tail = self.tail.load(Ordering::Acquire);
        if tail >= expected_tail {
            return Ok(tail);
        }
        Err(listener)
    }
}

#[async_trait]
impl Tracker for BroadcastTracker {
    fn new(mut size: usize) -> Self {
        size += 1;
        let mut counters = Vec::new();
        counters.resize_with(size, Default::default);
        Self {
            counters,
            num_receivers: Default::default(),
            tail: Default::default(),
            tail_move: Default::default(),
        }
    }

    fn new_receiver(&self) -> usize {
        self.num_receivers.fetch_add(1, Ordering::SeqCst);
        let tail_pos = self.tail.load(Ordering::Acquire);
        let index = tail_pos % self.counters.len();
        //TODO there is a race condition here!
        unsafe {
            self.counters
                .get_unchecked(index)
                .fetch_add(1, Ordering::SeqCst);
        }
        tail_pos
    }

    fn move_receiver(&self, from: usize, to: usize) {
        if to == from {
            return;
        }
        let from_idx = from % self.counters.len();
        let to_idx = to % self.counters.len();
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
            self.tail_move.notify(usize::MAX);
        }
    }

    fn remove_receiver(&self, at: usize) {
        let index = at % self.counters.len();
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

    fn slowest(&self) -> usize {
        self.tail.load(Ordering::Acquire)
    }

    fn wait_for_tail(&self, expected_tail: usize) -> usize {
        loop {
            let tail = self.tail.load(Ordering::Acquire);
            if tail >= expected_tail {
                return tail;
            }
            core::hint::spin_loop();
        }
        // loop {
        //     match self.wait_for_shared(expected_tail) {
        //         Ok(tail) => return tail,
        //         Err(listener) => listener.wait(),
        //     }
        // }
    }

    async fn wait_for_tail_async(&self, expected_tail: usize) -> usize {
        loop {
            match self.wait_for_shared(expected_tail) {
                Ok(tail) => return tail,
                Err(listener) => listener.await,
            }
        }
    }
}

#[cfg(test)]
mod tracker_tests {
    use super::*;
    use std::thread;
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
        let tracker = BroadcastTracker::new(10);
        let shared_cursor_a = tracker.new_receiver();
        tracker.move_receiver(shared_cursor_a, 4);

        thread::scope(|s| {
            let th = s.spawn(|| {
                assert_eq!(tracker.slowest(), 4);
                tracker.wait_for_tail(9);
                assert_eq!(tracker.slowest(), 9);
            });
            thread::sleep(Duration::from_millis(50));
            for i in 5..10 {
                tracker.move_receiver(i - 1, i);
                thread::sleep(Duration::from_millis(10));
            }
            th.join().expect("couldn't join");
        });
    }
}
