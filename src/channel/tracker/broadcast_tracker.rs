use std::collections::LinkedList;
use std::sync::atomic::{AtomicIsize, AtomicUsize, Ordering};
use std::sync::Arc;

use super::Tracker;

const UNUSED: usize = (isize::MAX as usize) + 1;

trait Cell {
    fn set_as_unused(&self);
    fn is_unused(&self) -> bool;
}

impl Cell for AtomicUsize {
    fn set_as_unused(&self) {
        self.store(UNUSED, Ordering::Release);
    }
    fn is_unused(&self) -> bool {
        self.load(Ordering::Acquire) == UNUSED
    }
}

#[derive(Debug)]
pub(crate) struct BroadcastTracker {
    // Access will always be write so no need for a more complex read write lock here.
    // It shouldn't be accessed too much and should only impede new/dying receivers not active
    // senders or receivers
    unused_cells: parking_lot::Mutex<LinkedList<usize>>,
    receiver_cells: parking_lot::RwLock<Vec<Arc<AtomicUsize>>>,
    slowest_cache: AtomicIsize,
}

impl Default for BroadcastTracker {
    fn default() -> Self {
        Self {
            unused_cells: Default::default(),
            receiver_cells: Default::default(),
            slowest_cache: AtomicIsize::new(-1),
        }
    }
}

impl Tracker for BroadcastTracker {
    type Token = usize;

    fn new_receiver(&self, at: isize) -> (Self::Token, Arc<AtomicUsize>) {
        let at = at as usize;
        //check for and claim an existing dead cell first
        let dead_idx;
        {
            let mut lock = self.unused_cells.lock();
            dead_idx = lock.pop_front();
        }

        if let Some(dead_idx) = dead_idx {
            // There is an existing dead slot that we should re-use
            let read_lock = self.receiver_cells.read();
            //TODO remove this expect
            let cell = read_lock.get(dead_idx).expect("couldn't get cell");
            cell.store(at, Ordering::Release);
            return (dead_idx, cell.clone());
        }

        // There are no available slots so we will have to create a new cell
        let new_cell = Arc::new(AtomicUsize::new(at));
        let cloned_new_cell = new_cell.clone();
        {
            let mut write_lock = self.receiver_cells.write();
            write_lock.push(new_cell);
            (write_lock.len() - 1, cloned_new_cell)
        }
    }

    fn remove_receiver(&self, token: Self::Token, cell: &Arc<AtomicUsize>) {
        {
            let mut lock = self.unused_cells.lock();
            lock.push_back(token);
        }
        cell.set_as_unused();
    }

    fn slowest(&self, min: isize) -> isize {
        let cached = self.slowest_cache.load(Ordering::Acquire);
        if cached > min {
            return cached;
        }
        let read_lock = self.receiver_cells.read();
        let mut slowest = isize::MAX;
        for cell in read_lock.iter() {
            //TODO Can we actually use relaxed here?
            let position = cell.load(Ordering::Relaxed);
            if position == UNUSED {
                continue;
            }
            let position = position as isize;
            if position <= min {
                return position;
            }
            if position < slowest {
                slowest = position;
            }
        }
        // cache this computation so that we may not have to do a scan next time!
        self.slowest_cache.store(slowest, Ordering::Release);
        slowest
    }
}

#[cfg(test)]
mod tracker_tests {
    use super::*;

    #[test]
    fn add_remove_recover() {
        let tracker = BroadcastTracker::default();
        let (receiver_idx, shared_cursor_a) = tracker.new_receiver(4);
        assert_eq!(receiver_idx, 0);
        assert_eq!(shared_cursor_a.load(Ordering::Acquire), 4);
        let (receiver_idx, shared_cursor) = tracker.new_receiver(6);
        assert_eq!(receiver_idx, 1);
        assert_eq!(shared_cursor.load(Ordering::Acquire), 6);
        tracker.remove_receiver(0, &shared_cursor_a);
        assert_eq!(shared_cursor_a.load(Ordering::Acquire), UNUSED);
        assert!(shared_cursor_a.is_unused()); // same as previous check just checks the function
        let (receiver_idx, shared_cursor) = tracker.new_receiver(7);
        assert_eq!(receiver_idx, 0);
        assert_eq!(shared_cursor.load(Ordering::Acquire), 7);
    }

    #[test]
    fn slowest() {
        let tracker = BroadcastTracker::default();
        let _ = tracker.new_receiver(4);
        let _ = tracker.new_receiver(6);
        let slowest = tracker.slowest(7);
        assert_eq!(slowest, 4);
        let _ = tracker.new_receiver(2);
        let slowest = tracker.slowest(7);
        assert_eq!(slowest, 4);
        let slowest = tracker.slowest(3);
        assert_eq!(slowest, 2);
        assert_eq!(tracker.slowest_cache.load(Ordering::Acquire), -1);
        let slowest = tracker.slowest(0);
        assert_eq!(slowest, 2);
        assert_eq!(tracker.slowest_cache.load(Ordering::Acquire), 2);
    }
}
