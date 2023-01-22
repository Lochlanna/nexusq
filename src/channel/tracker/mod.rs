pub mod broadcast_tracker;

use std::sync::atomic::AtomicUsize;
use std::sync::Arc;

pub trait Tracker {
    fn new_receiver(&self, at: isize) -> Arc<AtomicUsize>;
    //TODO better name for cell?
    fn remove_receiver(&self, cell: &Arc<AtomicUsize>);
    fn slowest(&self, min: isize) -> isize;
    fn tidy(&self);
    fn garbage_count(&self) -> usize;
}
