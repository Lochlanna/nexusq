use super::{ObservableCell, Tracker};
use std::collections::LinkedList;
use std::sync::atomic::{AtomicIsize, AtomicUsize, Ordering};
use std::sync::Arc;

#[derive(Debug)]
pub struct BroadcastTracker {
    // Access will always be write so no need for a more complex read write lock here.
    // It shouldn't be accessed too much and should only impede new/dying receivers not active
    // senders or receivers
    unused_cells: parking_lot::Mutex<LinkedList<Arc<ObservableCell>>>,
    receiver_cells: parking_lot::RwLock<Vec<Arc<ObservableCell>>>,
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
    fn new_receiver(&self, at: isize) -> Arc<ObservableCell> {
        let at = at as usize;
        //check for and claim an existing dead cell first
        let unused_cell;
        {
            let mut lock = self.unused_cells.lock();
            unused_cell = lock.pop_front();
        }
        if let Some(unused_cell) = unused_cell {
            unused_cell.store(at);
            return unused_cell;
        }

        // There are no available slots so we will have to create a new cell
        let new_cell = Arc::new(ObservableCell::at(at));
        let cloned_new_cell = new_cell.clone();
        {
            let mut write_lock = self.receiver_cells.write();
            write_lock.push(new_cell);
        }
        cloned_new_cell
    }

    fn remove_receiver(&self, cell: Arc<ObservableCell>) {
        cell.set_unused();
        {
            let mut lock = self.unused_cells.lock();
            lock.push_back(cell);
        }
    }

    fn slowest(&self, min: isize) -> (isize, Option<Arc<ObservableCell>>) {
        let cached = self.slowest_cache.load(Ordering::Acquire);
        if cached > min {
            return (cached, None);
        }
        let read_lock = self.receiver_cells.read();
        let mut slowest = isize::MAX;
        let mut slowest_cell = None;
        for cell in read_lock.iter() {
            //TODO Can we actually use relaxed here?
            let position = cell.load(Ordering::Relaxed);
            if position == super::UNUSED {
                continue;
            }
            let position = position as isize;
            if position <= min {
                return (position, Some(cell.clone()));
            }
            if position < slowest {
                slowest = position;
                slowest_cell = Some(cell);
            }
        }
        // cache this computation so that we may not have to do a scan next time!
        self.slowest_cache.store(slowest, Ordering::Release);
        if let Some(slowest_cell) = slowest_cell {
            return (slowest, Some(slowest_cell.clone()));
        }
        (slowest, None)
    }

    fn tidy(&self) {
        let mut unused_cell_lock = self.unused_cells.lock();
        unused_cell_lock.clear();
        let mut cell_write_lock = self.receiver_cells.write();
        cell_write_lock.retain(|v| !v.is_unused());
    }

    fn garbage_count(&self) -> usize {
        let unused_cell_lock = self.unused_cells.lock();
        unused_cell_lock.len()
    }
}

#[cfg(test)]
mod tracker_tests {
    use super::*;

    #[test]
    fn add_remove_receiver() {
        let tracker = BroadcastTracker::default();
        let shared_cursor_a = tracker.new_receiver(4);
        assert_eq!(shared_cursor_a.load(Ordering::Acquire), 4);
        let shared_cursor = tracker.new_receiver(6);
        assert_eq!(shared_cursor.load(Ordering::Acquire), 6);
        tracker.remove_receiver(shared_cursor_a.clone());
        assert_eq!(
            shared_cursor_a.load(Ordering::Acquire),
            super::super::UNUSED
        );
        assert!(shared_cursor_a.is_unused()); // same as previous check just checks the function
        let shared_cursor = tracker.new_receiver(7);
        assert_eq!(shared_cursor.load(Ordering::Acquire), 7);
    }

    #[test]
    fn slowest() {
        let tracker = BroadcastTracker::default();
        let _ = tracker.new_receiver(4);
        let _ = tracker.new_receiver(6);
        let slowest = tracker.slowest(7);
        assert_eq!(slowest.0, 4);
        let _ = tracker.new_receiver(2);
        let slowest = tracker.slowest(7);
        assert_eq!(slowest.0, 4);
        let slowest = tracker.slowest(3);
        assert_eq!(slowest.0, 2);
        assert_eq!(tracker.slowest_cache.load(Ordering::Acquire), -1);
        let slowest = tracker.slowest(0);
        assert_eq!(slowest.0, 2);
        assert_eq!(tracker.slowest_cache.load(Ordering::Acquire), 2);
    }
}
