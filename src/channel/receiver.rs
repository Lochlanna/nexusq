use super::*;
use crate::channel::tracker::{ObservableCell, Tracker};
use crate::BroadcastSender;
use async_trait::async_trait;
use core::slice;
use std::io;
use std::io::Write;
use std::mem::ManuallyDrop;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ReaderError {
    /// There is nothing new to be read from the channel
    #[error("there is no unread data on the channel")]
    NoNewData,
    /// The given destination vector is already full
    #[error("the destination for reads doesn't have remaining capacity")]
    DestinationFull,
}

#[async_trait]
pub trait Receiver {
    type Item;
    fn try_read_next(&mut self) -> Result<Self::Item, ReaderError>;
    fn recv(&mut self) -> Result<Self::Item, ReaderError>;
    async fn async_recv(&mut self) -> Result<Self::Item, ReaderError>;
    fn batch_recv(&mut self, res: &mut Vec<Self::Item>) -> Result<(), ReaderError>;
}

#[derive(Debug, Clone)]
pub struct BroadcastReceiver<T> {
    inner: ReceiverCore<T, BroadcastTracker>,
}

impl<T> From<ReceiverCore<T, BroadcastTracker>> for BroadcastReceiver<T> {
    fn from(inner: ReceiverCore<T, BroadcastTracker>) -> Self {
        Self { inner }
    }
}

impl<T> From<&BroadcastSender<T>> for BroadcastReceiver<T> {
    fn from(sender: &BroadcastSender<T>) -> Self {
        let core = sender.clone_core();
        Self { inner: core.into() }
    }
}

impl<T> BroadcastReceiver<T> {
    /// Creates a new receiver at the most recent entry in the stream
    pub fn add_stream(&self) -> Self {
        Self {
            inner: self.inner.add_stream(),
        }
    }

    pub(crate) fn clone_core(&self) -> Arc<Core<T, BroadcastTracker>> {
        self.inner.disruptor.clone()
    }

    /// Creates a new send handle to the same channel
    pub fn new_sender(&self) -> BroadcastSender<T> {
        self.into()
    }
}

#[async_trait]
impl<T> Receiver for BroadcastReceiver<T>
where
    T: Clone + Send,
{
    type Item = T;

    /// Try to read the next value from the channel. This function will not block and will return
    /// a [`ReaderError::NoNewData`] if there is no data available
    ///
    /// # Examples
    ///
    /// ```
    ///# use nexusq::{channel, Sender, Receiver};
    ///let (mut sender, mut receiver) = channel(10);
    ///sender.send(4).expect("couldn't send");
    ///let result = receiver.try_read_next().expect("couldn't receive");
    ///assert_eq!(result, 4);
    /// ```
    fn try_read_next(&mut self) -> Result<Self::Item, ReaderError> {
        self.inner.try_read_next()
    }
    /// Read the next value from the channel. This function will block and wait for data to
    /// become available.
    ///
    /// # Examples
    ///
    /// ```
    ///# use nexusq::{channel, Sender, Receiver};
    ///let (mut sender, mut receiver) = channel(10);
    ///sender.send(4).expect("couldn't send");
    ///let result = receiver.recv().expect("couldn't receive");
    ///assert_eq!(result, 4);
    /// ```
    fn recv(&mut self) -> Result<Self::Item, ReaderError> {
        self.inner.recv()
    }
    /// Async version of [`BroadcastReceiver::recv`]
    async fn async_recv(&mut self) -> Result<Self::Item, ReaderError> {
        self.inner.async_recv().await
    }
    /// Read as many values as are available. Results are stored in the argument `result`
    ///
    /// This operation is more efficient than reading values individually as it only has to
    /// check the cursors a single time.
    ///
    /// If T implements Copy this function will be extremely fast as the data is copied directly into
    /// the result array using memcpy.
    ///
    /// # Arguments
    ///
    /// * `result`: A pre allocated vector in which to store the results. The vector will not reallocate
    ///
    /// # Examples
    ///
    /// ```
    ///# use nexusq::{channel, Sender, Receiver};
    ///let (mut sender, mut receiver) = channel(10);
    ///let expected: Vec<i32> = (0..8).map(|v|{sender.send(v); v}).collect();
    ///let mut result = Vec::with_capacity(8);
    ///receiver.batch_recv(&mut result).expect("batch recv failed");
    ///assert_eq!(result, expected);
    /// ```
    fn batch_recv(&mut self, res: &mut Vec<Self::Item>) -> Result<(), ReaderError> {
        self.inner.batch_recv(res)
    }
}

#[derive(Debug)]
pub(crate) struct ReceiverCore<T, TR: Tracker> {
    disruptor: Arc<Core<T, TR>>,
    internal_cursor: isize,
    shared_cursor: ManuallyDrop<Arc<ObservableCell>>,
    capacity: isize,
}

impl<T, TR> Drop for ReceiverCore<T, TR>
where
    TR: Tracker,
{
    fn drop(&mut self) {
        unsafe {
            let shared_cursor = ManuallyDrop::take(&mut self.shared_cursor);
            self.disruptor.readers.remove_receiver(shared_cursor)
        }
    }
}

impl<T, TR> From<Arc<Core<T, TR>>> for ReceiverCore<T, TR>
where
    TR: Tracker,
{
    fn from(disruptor: Arc<Core<T, TR>>) -> Self {
        let shared_cursor = ManuallyDrop::new(
            disruptor.readers.new_receiver(
                disruptor
                    .committed
                    .load(Ordering::Acquire)
                    .clamp(0, isize::MAX),
            ),
        );
        let capacity = disruptor.capacity;
        Self {
            disruptor,
            internal_cursor: shared_cursor.load(Ordering::Relaxed) as isize,
            shared_cursor,
            capacity,
        }
    }
}

impl<T, TR> Clone for ReceiverCore<T, TR>
where
    TR: Tracker,
{
    /// Creates a new receiver at the same point in the stream
    fn clone(&self) -> Self {
        let shared_cursor =
            ManuallyDrop::new(self.disruptor.readers.new_receiver(self.internal_cursor));
        Self {
            disruptor: self.disruptor.clone(),
            internal_cursor: shared_cursor.load(Ordering::Relaxed) as isize,
            shared_cursor,
            capacity: self.capacity,
        }
    }
}

impl<T, TR> ReceiverCore<T, TR>
where
    TR: Tracker,
{
    #[inline(always)]
    fn increment_cursor(&mut self) {
        self.internal_cursor += 1;
        self.shared_cursor.store(self.internal_cursor as usize);
    }
}

impl<T, TR> ReceiverCore<T, TR>
where
    TR: Tracker,
{
    /// Creates a new receiver at the most recent entry in the stream
    pub fn add_stream(&self) -> Self {
        self.disruptor.clone().into()
    }
}

impl<T, TR> ReceiverCore<T, TR>
where
    T: Clone,
    TR: Tracker,
{
    /// Try to read the next value from the channel. This function will not block and will return
    /// a [`ReaderError::NoNewData`] if there is no data available
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
        Ok(value)
    }

    /// Read the next value from the channel. This function will block and wait for data to
    /// become available.
    pub fn recv(&mut self) -> Result<T, ReaderError> {
        let immediate = self.try_read_next();
        match &immediate {
            Ok(_) => return immediate,
            Err(err) => {
                if !matches!(err, ReaderError::NoNewData) {
                    return immediate;
                }
            }
        };

        loop {
            let listener = self.disruptor.writer_move.listen();
            // try again as the listener can take some time to be registered and cause us to miss things
            let immediate = self.try_read_next();
            match &immediate {
                Ok(_) => return immediate,
                Err(err) => {
                    if !matches!(err, ReaderError::NoNewData) {
                        return immediate;
                    }
                }
            };
            listener.wait();

            let immediate = self.try_read_next();
            match &immediate {
                Ok(_) => return immediate,
                Err(err) => {
                    if !matches!(err, ReaderError::NoNewData) {
                        return immediate;
                    }
                }
            };
        }
    }

    /// Read as many values as are available. Results are stored in the argument `result`
    ///
    /// This operation is more efficient than reading values individually as it only has to
    /// check the cursors a single time.
    ///
    /// If T implements Copy this function will be extremely fast as the data is copied directly into
    /// the result array using memcpy.
    ///
    /// # Arguments
    ///
    /// * `result`: A pre allocated vector in which to store the results. The vector will not reallocate
    pub fn batch_recv(&mut self, result: &mut Vec<T>) -> Result<(), ReaderError> {
        let result_remaining_capacity = result.capacity() - result.len();
        if result_remaining_capacity == 0 {
            return Err(ReaderError::DestinationFull);
        }
        let committed = self.disruptor.committed.load(Ordering::Acquire);
        if self.internal_cursor > committed {
            return Err(ReaderError::NoNewData);
        }
        let num_available = (committed + 1 - self.internal_cursor) as usize;
        let num_to_read = num_available.clamp(0, result.capacity() - result.len());
        if num_to_read == 0 {
            return Err(ReaderError::NoNewData);
        }
        result.reserve_exact(num_to_read);
        let from_index = (self.internal_cursor % self.capacity) as usize;
        let to_index = from_index + num_to_read;
        if to_index <= self.capacity as usize {
            // this copy won't wrap
            unsafe {
                let target =
                    slice::from_raw_parts_mut(result.as_mut_ptr().add(result.len()), num_to_read);
                let src = &(*self.disruptor.ring)[from_index..to_index];
                result.set_len(result.len() + target.len());
                // Clone will specialise to copy when it can. This will be very fast for copy types!
                target.clone_from_slice(src);
            }
        } else {
            //this copy will wrap!
            // end section
            unsafe {
                let target = slice::from_raw_parts_mut(
                    result.as_mut_ptr().add(result.len()),
                    self.capacity as usize - from_index,
                );
                let src = &(*self.disruptor.ring)[from_index..];
                result.set_len(result.len() + target.len());
                // Clone will specialise to copy when it can. This will be very fast for copy types!
                target.clone_from_slice(src);
            }
            // start section
            unsafe {
                let target = slice::from_raw_parts_mut(
                    result.as_mut_ptr().add(result.len()),
                    to_index - self.capacity as usize,
                );
                let src = &(*self.disruptor.ring)[..(to_index - (self.capacity as usize))];
                result.set_len(result.len() + target.len());
                // Clone will specialise to copy when it can. This will be very fast for copy types!
                target.clone_from_slice(src);
            }
        }
        Ok(())
    }
}

impl<T, TR> ReceiverCore<T, TR>
where
    T: Clone + Send,
    TR: Tracker,
{
    /// Async version of [`ReceiverCore::recv`]
    pub async fn async_recv(&mut self) -> Result<T, ReaderError> {
        let immediate = self.try_read_next();
        match &immediate {
            Ok(_) => return immediate,
            Err(err) => {
                if !matches!(err, ReaderError::NoNewData) {
                    return immediate;
                }
            }
        };

        loop {
            let listener = self.disruptor.writer_move.listen();
            // try again as the listener can take some time to be registered and cause us to miss things
            let immediate = self.try_read_next();
            match &immediate {
                Ok(_) => return immediate,
                Err(err) => {
                    if !matches!(err, ReaderError::NoNewData) {
                        return immediate;
                    }
                }
            };
            listener.await;

            let immediate = self.try_read_next();
            match &immediate {
                Ok(_) => return immediate,
                Err(err) => {
                    if !matches!(err, ReaderError::NoNewData) {
                        return immediate;
                    }
                }
            };
        }
    }
}

#[cfg(test)]
mod receiver_tests {
    use crate::channel::sender::Sender;
    use crate::channel::*;
    use crate::{BroadcastReceiver, Receiver};

    #[test]
    fn batch_read() {
        let (mut sender, mut receiver) = channel(10);
        let expected: Vec<i32> = (0..8).collect();
        for i in &expected {
            sender.send(*i).expect("couldn't send");
        }
        let mut result = Vec::with_capacity(8);
        receiver
            .batch_recv(&mut result)
            .expect("receiver was okay!");

        assert_eq!(result, expected)
    }

    #[test]
    fn batch_read_entire_buffer() {
        let (mut sender, mut receiver) = channel(10);
        let expected: Vec<i32> = (0..10).collect();
        for i in &expected {
            sender.send(*i).expect("couldn't send");
        }
        let mut result = Vec::with_capacity(10);
        receiver
            .batch_recv(&mut result)
            .expect("receiver was okay!");

        assert_eq!(result, expected)
    }

    #[test]
    fn batch_read_capacity_limited() {
        let (mut sender, mut receiver) = channel(10);
        let expected: Vec<i32> = (0..10).collect();
        for i in &expected {
            sender.send(*i).expect("couldn't send");
        }
        let mut result = Vec::with_capacity(3);
        receiver
            .batch_recv(&mut result)
            .expect("receiver was okay!");
        let expected: Vec<i32> = (0..3).collect();
        assert_eq!(result, expected)
    }

    #[test]
    fn wrapping_batch_read() {
        let (mut sender, mut receiver) = channel(10);
        for i in 0..10 {
            sender.send(i).expect("couldn't send");
        }
        let _ = receiver.recv();
        let _ = receiver.recv();
        sender.send(10).expect("couldn't send");
        sender.send(11).expect("couldn't send");
        let expected: Vec<i32> = (2..12).collect();
        let mut result = Vec::with_capacity(10);
        receiver
            .batch_recv(&mut result)
            .expect("receiver was okay!");

        assert_eq!(result, expected)
    }

    #[test]
    fn receiver_from_sender() {
        let (mut sender, _) = channel(10);
        sender.send(42).expect("couldn't send");
        let mut receiver: BroadcastReceiver<i32> = (&sender).into();
        let v = receiver.recv().expect("couldn't receive");
        assert_eq!(v, 42);
    }
}
