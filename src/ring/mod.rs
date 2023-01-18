use crossbeam_skiplist::SkipSet;
use std::cmp::Ordering;
use std::intrinsics::forget;
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
struct DisruptorCore<T> {
    ring: *mut Vec<T>,
    buffer_size: usize,
    claimed: AtomicIsize,
    committed: AtomicIsize,
    next_reader_id: AtomicUsize,
    // Reference to each reader to get their position. It should be sorted(how..?)
    readers: SkipSet<ReadCursor>,
}

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
        unsafe {
            ring.set_len(buffer_size);
        }

        let ring = Box::into_raw(ring);

        Self {
            ring,
            buffer_size,
            claimed: AtomicIsize::new(0),
            committed: AtomicIsize::new(-1),
            next_reader_id: AtomicUsize::default(),
            readers: Default::default(),
        }
    }
}
