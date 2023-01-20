use std::collections::LinkedList;
use std::sync::atomic::{AtomicIsize, AtomicUsize, Ordering};
use std::sync::Arc;

const DEAD_CELL: usize = (isize::MAX as usize) + 1;

trait Cell {
    fn kill(&self);
    fn is_dead(&self) -> bool;
}

impl Cell for AtomicUsize {
    fn kill(&self) {
        self.store(DEAD_CELL, Ordering::Release);
    }
    fn is_dead(&self) -> bool {
        self.load(Ordering::Acquire) == DEAD_CELL
    }
}

#[derive(Debug)]
pub(crate) struct ReceiverTracker {
    // Access will always be write so no need for a more complex read write lock here.
    // It shouldn't be accessed too much and should only impede new/dying receivers not active
    // senders or receivers
    dead_cells: parking_lot::Mutex<LinkedList<usize>>,
    receiver_cells: parking_lot::RwLock<Vec<Arc<AtomicUsize>>>,
    slowest_cache: AtomicIsize,
}

impl Default for ReceiverTracker {
    fn default() -> Self {
        Self {
            dead_cells: Default::default(),
            receiver_cells: Default::default(),
            slowest_cache: AtomicIsize::new(-1),
        }
    }
}

impl ReceiverTracker {
    pub fn new_receiver(&self, at: isize) -> (usize, Arc<AtomicUsize>) {
        let at = at as usize;
        //check for and claim an existing dead cell first
        let dead_idx;
        {
            let mut lock = self.dead_cells.lock();
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
            return (write_lock.len() - 1, cloned_new_cell);
        }
    }

    pub fn kill_receiver(&self, index: usize, cell: &Arc<AtomicUsize>) {
        {
            //
            let mut lock = self.dead_cells.lock();
            lock.push_back(index);
        }
        cell.kill();
    }

    pub fn slowest(&self, min: isize) -> isize {
        let cached = self.slowest_cache.load(Ordering::Acquire);
        if cached > min {
            return cached;
        }
        let read_lock = self.receiver_cells.read();
        let mut slowest = isize::MAX;
        for cell in read_lock.iter() {
            //TODO Can we actually use relaxed here?
            let position = cell.load(Ordering::Relaxed);
            if position == DEAD_CELL {
                continue;
            }
            let position = position as isize;
            if position <= min {
                return position as isize;
            }
            if position < slowest {
                slowest = position as isize;
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
        let tracker = ReceiverTracker::default();
        let (receiver_idx, shared_cursor_a) = tracker.new_receiver(4);
        assert_eq!(receiver_idx, 0);
        assert_eq!(shared_cursor_a.load(Ordering::Acquire), 4);
        let (receiver_idx, shared_cursor) = tracker.new_receiver(6);
        assert_eq!(receiver_idx, 1);
        assert_eq!(shared_cursor.load(Ordering::Acquire), 6);
        tracker.kill_receiver(0, &shared_cursor_a);
        assert_eq!(shared_cursor_a.load(Ordering::Acquire), DEAD_CELL);
        assert!(shared_cursor_a.is_dead()); // same as previous check just checks the function
        let (receiver_idx, shared_cursor) = tracker.new_receiver(7);
        assert_eq!(receiver_idx, 0);
        assert_eq!(shared_cursor.load(Ordering::Acquire), 7);
    }

    #[test]
    fn slowest() {
        let tracker = ReceiverTracker::default();
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
