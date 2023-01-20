use super::*;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ReaderError {
    #[error("there is no unread data on the channel")]
    NoNewData,
}

#[derive(Debug)]
pub struct Receiver<T> {
    disruptor: Arc<Core<T>>,
    internal_cursor: isize,
    shared_cursor: Arc<AtomicUsize>,
    shared_cursor_id: usize,
}

impl<T> Drop for Receiver<T> {
    fn drop(&mut self) {
        self.disruptor
            .readers
            .kill_receiver(self.shared_cursor_id, &self.shared_cursor)
    }
}

impl<T> From<Arc<Core<T>>> for Receiver<T> {
    fn from(disruptor: Arc<Core<T>>) -> Self {
        let (shared_cursor_id, shared_cursor) = disruptor.readers.new_receiver(
            disruptor
                .committed
                .load(Ordering::Acquire)
                .clamp(0, isize::MAX),
        );
        Self {
            disruptor,
            internal_cursor: shared_cursor.load(Ordering::Relaxed) as isize,
            shared_cursor,
            shared_cursor_id,
        }
    }
}

impl<T> Clone for Receiver<T> {
    fn clone(&self) -> Self {
        self.disruptor.clone().into()
    }
}

impl<T> Receiver<T> {
    #[inline(always)]
    fn increment_cursor(&mut self) {
        self.internal_cursor += 1;
        self.shared_cursor
            .store(self.internal_cursor as usize, Ordering::Release);
    }
}

impl<T> Receiver<T>
where
    T: Clone,
{
    pub fn try_read_next(&mut self) -> Result<T, ReaderError> {
        let committed = self.disruptor.committed.load(Ordering::Acquire);
        if self.internal_cursor > committed {
            return Err(ReaderError::NoNewData);
        }
        let index = self.internal_cursor as usize % self.disruptor.capacity as usize;
        // the value has been committed so it's safe to read it!
        let value;
        unsafe {
            value = (*self.disruptor.ring).get_unchecked(index).clone();
        }
        self.increment_cursor();
        self.disruptor.reader_move.notify(usize::MAX);
        Ok(value)
    }
}
