use crossbeam_skiplist::SkipSet;
use event_listener::Event;
use std::cmp::Ordering;
use std::sync::atomic::{AtomicIsize, AtomicUsize};

pub mod receiver;
pub mod sender;

#[derive(Debug, Eq, PartialEq, Copy, Clone, Hash)]
struct ReadCursor {
    reader_id: usize,
    current_id: isize,
}

impl ReadCursor {
    fn new(reader_id: usize) -> Self {
        Self {
            reader_id,
            current_id: 0,
        }
    }
    fn increment(&mut self) {
        self.current_id += 1;
    }

    fn reader_id(&self) -> usize {
        self.reader_id
    }
    fn current_id(&self) -> isize {
        self.current_id
    }
}

impl PartialOrd for ReadCursor {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ReadCursor {
    fn cmp(&self, other: &Self) -> Ordering {
        let ord = self.current_id.cmp(&other.current_id);
        if matches!(ord, Ordering::Equal) {
            return self.reader_id.cmp(&other.reader_id);
        }
        ord
    }
}

#[derive(Debug)]
pub(crate) struct DisruptorCore<T> {
    ring: *mut Vec<T>,
    capacity: usize,
    claimed: AtomicIsize,
    committed: AtomicIsize,
    next_reader_id: AtomicUsize,
    // is there a better way than events?
    reader_move: Event,
    writer_move: Event,
    // Reference to each reader to get their position. It should be sorted(how..?)
    readers: SkipSet<ReadCursor>,
}

unsafe impl<T> Send for DisruptorCore<T> {}
unsafe impl<T> Sync for DisruptorCore<T> {}

impl<T> Drop for DisruptorCore<T> {
    fn drop(&mut self) {
        unsafe {
            drop(Box::from_raw(self.ring));
        }
    }
}

impl<T> DisruptorCore<T> {
    pub(crate) fn new(buffer_size: usize) -> Self {
        let mut ring = Box::new(Vec::with_capacity(buffer_size));
        let capacity = ring.capacity();
        unsafe {
            // use capacity as vec is allowed to allocate more than buffer_size if it likes so
            //we might as well use it!
            ring.set_len(capacity);
        }

        let ring = Box::into_raw(ring);

        Self {
            ring,
            capacity,
            claimed: AtomicIsize::new(0),
            committed: AtomicIsize::new(-1),
            next_reader_id: AtomicUsize::default(),
            reader_move: Default::default(),
            writer_move: Default::default(),
            readers: Default::default(),
        }
    }
}
