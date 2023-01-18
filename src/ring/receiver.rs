use super::*;
use std::sync::atomic::Ordering as AOrdering;
use std::sync::Arc;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ReaderError {
    #[error("there is no unread data on the channel")]
    NoNewData,
}

#[derive(Debug)]
pub struct Receiver<T> {
    disruptor: Arc<DisruptorCore<T>>,
    cursor: ReadCursor,
}

impl<T> Receiver<T> {
    #[inline]
    fn increment_cursor(&mut self) {
        // make a copy of our current cursor so that we can remove it later
        let previous = self.cursor;
        self.cursor.increment();
        self.disruptor.readers.insert(self.cursor);
        // We can only remove the old one once we have the new one in place to prevent a writer from
        // claiming our new position
        self.disruptor.readers.remove(&previous);
    }
}

impl<T> Receiver<T>
where
    T: Clone,
{
    pub fn try_read_next(&mut self) -> Result<T, ReaderError> {
        let committed = self.disruptor.committed.load(AOrdering::Acquire);
        if self.cursor.current_id > committed {
            return Err(ReaderError::NoNewData);
        }
        let index = self.cursor.current_id as usize % self.disruptor.buffer_size;
        // the value has been committed so it's safe to read it!
        let value;
        unsafe {
            value = (*self.disruptor.ring).get_unchecked(index).clone();
        }
        self.increment_cursor();
        Ok(value)
    }
}
