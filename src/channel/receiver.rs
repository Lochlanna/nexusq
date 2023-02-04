use super::*;
use crate::channel::tracker::Tracker;
use crate::BroadcastSender;
use async_trait::async_trait;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ReaderError {
    /// There is nothing new to be read from the channel
    #[error("there is no unread data on the channel")]
    NoNewData,
}

#[async_trait]
pub trait Receiver {
    type Item;
    fn recv(&mut self) -> Result<Self::Item, ReaderError>;
    async fn async_recv(&mut self) -> Result<Self::Item, ReaderError>;
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
        Ok(self.inner.recv())
    }
    /// Async version of [`BroadcastReceiver::recv`]
    async fn async_recv(&mut self) -> Result<Self::Item, ReaderError> {
        Ok(self.inner.async_recv().await)
    }
}

#[derive(Debug)]
pub(crate) struct ReceiverCore<T, TR: Tracker> {
    disruptor: Arc<Core<T, TR>>,
    internal_cursor: usize,
    capacity: usize,
}

impl<T, TR> Drop for ReceiverCore<T, TR>
where
    TR: Tracker,
{
    fn drop(&mut self) {
        self.disruptor.readers.remove_receiver(self.internal_cursor);
    }
}

impl<T, TR> From<Arc<Core<T, TR>>> for ReceiverCore<T, TR>
where
    TR: Tracker,
{
    fn from(disruptor: Arc<Core<T, TR>>) -> Self {
        let internal_cursor = disruptor.readers.new_receiver();
        let capacity = disruptor.capacity;
        Self {
            disruptor,
            internal_cursor,
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
        let tail = self.disruptor.readers.new_receiver();
        self.disruptor
            .readers
            .move_receiver(tail, self.internal_cursor);
        Self {
            disruptor: self.disruptor.clone(),
            internal_cursor: self.internal_cursor,
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
        self.disruptor
            .readers
            .move_receiver(self.internal_cursor - 1, self.internal_cursor)
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
    fn try_read_next(&mut self) -> Result<T, ReaderError> {
        let committed = self.disruptor.committed.load(Ordering::Acquire);
        if committed < 0 || self.internal_cursor > committed as usize {
            return Err(ReaderError::NoNewData);
        }
        let index = self.internal_cursor % self.capacity;
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
    pub fn recv(&mut self) -> T {
        loop {
            // try again as the listener can take some time to be registered and cause us to miss things
            if let Ok(message) = self.try_read_next() {
                return message;
            }
            let listener = self.disruptor.writer_move.listen();
            if let Ok(message) = self.try_read_next() {
                return message;
            }
            listener.wait();
        }
    }
}

impl<T, TR> ReceiverCore<T, TR>
where
    T: Clone + Send,
    TR: Tracker,
{
    /// Async version of [`ReceiverCore::recv`]
    pub async fn async_recv(&mut self) -> T {
        loop {
            // try again as the listener can take some time to be registered and cause us to miss things
            if let Ok(message) = self.try_read_next() {
                return message;
            }
            let listener = self.disruptor.writer_move.listen();
            if let Ok(message) = self.try_read_next() {
                return message;
            }
            listener.await
        }
    }
}

#[cfg(test)]
mod receiver_tests {
    use crate::channel::sender::Sender;
    use crate::channel::*;
    use crate::{BroadcastReceiver, Receiver};

    #[test]
    fn receiver_from_sender() {
        let (mut sender, _) = channel(10);
        sender.send(42).expect("couldn't send");
        let mut receiver: BroadcastReceiver<i32> = (&sender).into();
        let v = receiver.recv().expect("couldn't receive");
        assert_eq!(v, 42);
    }
}
