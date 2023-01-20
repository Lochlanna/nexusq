pub mod receiver;
mod receiver_tracker;
pub mod sender;

use event_listener::Event;
use receiver_tracker::ReceiverTracker;
use std::sync::atomic::{AtomicIsize, AtomicUsize};
use std::sync::Arc;

#[derive(Debug)]
pub(crate) struct Core<T> {
    ring: *mut Vec<T>,
    capacity: isize,
    claimed: AtomicIsize,
    committed: AtomicIsize,
    // is there a better way than events?
    reader_move: Event,
    writer_move: Event,
    // Reference to each reader to get their position. It should be sorted(how..?)
    readers: ReceiverTracker,
}

unsafe impl<T> Send for Core<T> {}
unsafe impl<T> Sync for Core<T> {}

impl<T> Drop for Core<T> {
    fn drop(&mut self) {
        unsafe {
            drop(Box::from_raw(self.ring));
        }
    }
}

impl<T> Core<T> {
    pub(crate) fn new(buffer_size: usize) -> Self {
        let mut ring = Box::new(Vec::with_capacity(buffer_size));
        let capacity = ring.capacity();
        //TODO check that it's not bigger than isize::MAX
        unsafe {
            // use capacity as vec is allowed to allocate more than buffer_size if it likes so
            //we might as well use it!
            ring.set_len(capacity);
        }

        let ring = Box::into_raw(ring);

        Self {
            ring,
            capacity: capacity as isize,
            claimed: AtomicIsize::new(0),
            committed: AtomicIsize::new(-1),
            reader_move: Default::default(),
            writer_move: Default::default(),
            readers: Default::default(),
        }
    }
}

pub fn channel<T>(size: usize) -> (sender::Sender<T>, receiver::Receiver<T>) {
    let core = Arc::new(Core::new(size));
    let sender = sender::Sender::from(core.clone());
    let receiver = receiver::Receiver::from(core);
    (sender, receiver)
}
