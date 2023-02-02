use super::Tracker;
use async_trait::async_trait;
use std::sync::atomic::{AtomicUsize, Ordering};

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
        todo!();
        let mut id = from;
        loop {
            //TODO maybe there is an optimisation here...
            let cell;
            unsafe {
                cell = self.counters.get_unchecked(id % self.counters.len());
            }
            if cell.load(Ordering::SeqCst) > 0 {
                if cell.fetch_add(1, Ordering::SeqCst) > 0 {
                    self.tail.store(id, Ordering::SeqCst);
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
}

#[async_trait]
impl Tracker for BroadcastTracker {
    fn new(size: usize) -> Self {
        let mut counters = vec![];
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
        let tail_pos = self.tail.load(Ordering::SeqCst);
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
        debug_assert!(to > from || to - from == 1);
        let from_idx = from % self.counters.len();
        let to_idx = to % self.counters.len();
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
        debug_assert!(previous > 0 && previous <= 2);
        //Assume there is at least one receiver so don't need to check here
        let tail = self.tail.load(Ordering::SeqCst);
        if previous == 1 && from == tail {
            if to - from == 1 {
                //we know we are the tail so just set and go!
                self.tail.store(to, Ordering::SeqCst);
                self.tail_move.notify(usize::MAX);
                return;
            }
            // we have skipped ahead so move forward until we find and manage to set the tail
            self.chase_tail(from);
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
        if previous == 1 && at == self.tail.load(Ordering::SeqCst) && num_left > 0 {
            // we have just removed the tail receiver so we need to chase the tail to find it
            self.chase_tail(at);
        }
    }

    fn slowest(&self) -> usize {
        self.tail.load(Ordering::SeqCst)
    }

    fn wait_for_tail(&self, expected_tail: usize) -> usize {
        loop {
            let mut tail = self.tail.load(Ordering::SeqCst);
            if tail >= expected_tail {
                return tail;
            }
            let listener = self.tail_move.listen();
            tail = self.tail.load(Ordering::SeqCst);
            if self.tail.load(Ordering::SeqCst) >= expected_tail {
                return tail;
            }
            listener.wait()
        }
    }

    async fn wait_for_tail_async(&self, expected_tail: usize) -> usize {
        loop {
            let mut tail = self.tail.load(Ordering::SeqCst);
            if tail >= expected_tail {
                return tail;
            }
            let listener = self.tail_move.listen();
            tail = self.tail.load(Ordering::SeqCst);
            if self.tail.load(Ordering::SeqCst) >= expected_tail {
                return tail;
            }
            listener.await
        }
    }
}

#[cfg(test)]
mod tracker_tests {
    use super::*;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn add_remove_receiver() {
        let tracker = BroadcastTracker::new(10);
        let shared_cursor_a = tracker.new_receiver();
        assert_eq!(tracker.counters[0].load(Ordering::SeqCst), 1);
        tracker.move_receiver(shared_cursor_a, 4);
        assert_eq!(tracker.counters[0].load(Ordering::SeqCst), 0);
        assert_eq!(tracker.counters[4].load(Ordering::SeqCst), 1);
        assert_eq!(tracker.slowest(), 4);

        let shared_cursor_b = tracker.new_receiver();
        tracker.move_receiver(shared_cursor_b, 6);
        assert_eq!(tracker.counters[6].load(Ordering::SeqCst), 1);
        assert_eq!(tracker.slowest(), 4);

        tracker.remove_receiver(4);
        assert_eq!(tracker.counters[4].load(Ordering::SeqCst), 0);
        assert_eq!(tracker.slowest(), 6);

        tracker.move_receiver(6, 7);
        assert_eq!(tracker.counters[6].load(Ordering::SeqCst), 0);
        assert_eq!(tracker.counters[7].load(Ordering::SeqCst), 1);
        assert_eq!(tracker.slowest(), 7);
    }

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
