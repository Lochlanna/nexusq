use super::*;
use core::slice;
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
    capacity: isize,
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
        let capacity = disruptor.capacity;
        Self {
            disruptor,
            internal_cursor: shared_cursor.load(Ordering::Relaxed) as isize,
            shared_cursor,
            shared_cursor_id,
            capacity,
        }
    }
}

impl<T> Clone for Receiver<T> {
    /// Creates a new receiver at the same point in the stream
    fn clone(&self) -> Self {
        let (shared_cursor_id, shared_cursor) =
            self.disruptor.readers.new_receiver(self.internal_cursor);
        Self {
            disruptor: self.disruptor.clone(),
            internal_cursor: shared_cursor.load(Ordering::Relaxed) as isize,
            shared_cursor,
            shared_cursor_id,
            capacity: self.capacity,
        }
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
    /// Creates a new receiver at the most recent entry in the stream
    pub fn add_stream(&self) -> Self {
        self.disruptor.clone().into()
    }
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

    pub fn recv(&mut self) -> Result<T, ReaderError> {
        let immediate = self.try_read_next();
        match &immediate {
            Ok(_) => return immediate,
            Err(err) => match err {
                ReaderError::NoNewData => {} // this is an expected error here
            },
        };

        let listener = self.disruptor.writer_move.listen();
        // try again as the listener can take some time to be registered and cause us to miss things
        let immediate = self.try_read_next();
        match &immediate {
            Ok(_) => return immediate,
            Err(err) => match err {
                ReaderError::NoNewData => {} // this is an expected error here
            },
        };
        listener.wait();

        loop {
            //TODO this loop shouldn't be needed here. What's going on...
            let immediate = self.try_read_next();
            match &immediate {
                Ok(_) => return immediate,
                Err(err) => match err {
                    ReaderError::NoNewData => continue, // this is an expected error here
                },
            };
        }
    }

    pub async fn async_recv(&mut self) -> Result<T, ReaderError> {
        let immediate = self.try_read_next();
        match &immediate {
            Ok(_) => return immediate,
            Err(err) => match err {
                ReaderError::NoNewData => {} // this is an expected error here
            },
        };

        let listener = self.disruptor.writer_move.listen();
        // try again as the listener can take some time to be registered and cause us to miss things
        let immediate = self.try_read_next();
        match &immediate {
            Ok(_) => return immediate,
            Err(err) => match err {
                ReaderError::NoNewData => {} // this is an expected error here
            },
        };
        listener.await;

        loop {
            //TODO this loop shouldn't be needed here. What's going on...
            let immediate = self.try_read_next();
            match &immediate {
                Ok(_) => return immediate,
                Err(err) => match err {
                    ReaderError::NoNewData => continue, // this is an expected error here
                },
            };
        }
    }

    pub fn batch_recv(&mut self, res: &mut Vec<T>) -> Result<(), ReaderError> {
        let committed = self.disruptor.committed.load(Ordering::Acquire);
        if self.internal_cursor > committed {
            return Err(ReaderError::NoNewData);
        }
        let num_to_read = (committed + 1 - self.internal_cursor) as usize;
        res.reserve_exact(num_to_read);
        let from_index = (self.internal_cursor % self.capacity) as usize;
        let to_index = from_index + num_to_read;
        if to_index <= self.capacity as usize {
            // this copy won't wrap
            unsafe {
                let target =
                    slice::from_raw_parts_mut(res.as_mut_ptr().add(res.len()), num_to_read);
                let src = &(*self.disruptor.ring)[from_index..to_index];
                res.set_len(res.len() + target.len());
                // Clone will specialise to copy when it can. This will be very fast for copy types!
                target.clone_from_slice(src);
            }
        } else {
            //this copy will wrap!
            // end section
            unsafe {
                let target = slice::from_raw_parts_mut(
                    res.as_mut_ptr().add(res.len()),
                    self.capacity as usize - from_index,
                );
                let src = &(*self.disruptor.ring)[from_index..];
                res.set_len(res.len() + target.len());
                // Clone will specialise to copy when it can. This will be very fast for copy types!
                target.clone_from_slice(src);
            }
            // start section
            unsafe {
                let target = slice::from_raw_parts_mut(
                    res.as_mut_ptr().add(res.len()),
                    to_index - self.capacity as usize,
                );
                let src = &(*self.disruptor.ring)[..(to_index - (self.capacity as usize))];
                res.set_len(res.len() + target.len());
                // Clone will specialise to copy when it can. This will be very fast for copy types!
                target.clone_from_slice(src);
            }
        }
        Ok(())
    }
}
