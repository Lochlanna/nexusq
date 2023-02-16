mod broadcast_tracker;
mod sequential_producer_tracker;

use thiserror::Error as ThisError;

pub use broadcast_tracker::MultiCursorTracker;
pub use sequential_producer_tracker::SequentialProducerTracker;

#[derive(ThisError, Debug)]
pub enum TrackerError {
    #[error("size must be a power of 2")]
    InvalidSize,
    #[error("the requested position no longer exists")]
    PositionTooOld,
}

pub trait Tracker {
    fn wait_for(&self, expected: isize) -> isize;
    fn current(&self) -> isize;
}

pub trait ReceiverTracker: Tracker {
    fn register(&self, at: isize) -> Result<isize, TrackerError>;
    fn update(&self, from: isize, to: isize);
    fn de_register(&self, at: isize);
}

pub trait ProducerTracker: Tracker {
    fn make_claim(&self) -> isize;
    fn publish(&self, id: isize);
}
