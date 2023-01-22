mod broadcast_tracker;

use std::sync::atomic::AtomicUsize;
use std::sync::Arc;

pub(crate) use broadcast_tracker::*;

pub(crate) trait Tracker {
    type Token;
    fn new_receiver(&self, at: isize) -> (Self::Token, Arc<AtomicUsize>);
    //TODO better name for cell?
    fn remove_receiver(&self, token: Self::Token, cell: &Arc<AtomicUsize>);
    fn slowest(&self, min: isize) -> isize;
}
