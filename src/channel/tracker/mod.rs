mod broadcast_tracker;
mod sequential_producer_tracker;

pub use broadcast_tracker::MultiCursorTracker;
pub use sequential_producer_tracker::SequentialProducerTracker;

use thiserror::Error as ThisError;

#[derive(ThisError, Debug)]
pub enum Error {
    #[error("size must be a power of 2")]
    InvalidSize,
}

pub trait ReceiverTracker {
    fn register(&self, at: isize) -> isize;
    fn update(&self, from: isize, to: isize);
    fn de_register(&self, at: isize);
}

pub trait ProducerTracker {
    fn make_claim(&self) -> isize;
    fn publish(&self, id: isize);
}

pub trait Tracker {
    fn wait_for(&self, expected: isize) -> isize;
    fn current(&self) -> isize;
}
