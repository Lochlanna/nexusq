use super::*;
use crate::channel::tracker::broadcast_tracker::BroadcastTracker;
use crate::channel::tracker::Tracker;
use crate::BroadcastReceiver;
use async_trait::async_trait;
use std::mem::forget;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SenderError {
    /// The given input is too large to fit in the buffered channel
    #[error("buffer not big enough to accept input")]
    InputTooLarge,
}

#[async_trait]
pub trait Sender {
    type Item: Send;
    /// Send a single value to the channel. This function will block if there is no space
    /// available in the channel.
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
    fn send(&mut self, value: Self::Item) -> Result<(), SenderError>;
    /// Send multiple values to the channel in one call. This function should perform better
    /// than sending them individually.
    ///
    /// This function will block until there is enough space in the channel to send all the values
    /// at once.
    ///
    /// # Arguments
    ///
    /// * `values`: A list of values to send to the channel.
    /// The length of this list must be <= the channel buffer size
    ///
    /// # Examples
    ///
    /// ```
    ///# use nexusq::{channel, Sender, Receiver};
    ///let (mut sender, mut receiver) = channel(50);
    ///let expected: Vec<i32> = (0..12).collect();
    ///sender.send_batch(expected.clone()).expect("batch send failed");
    ///let mut result = Vec::with_capacity(12);
    ///receiver
    ///    .batch_recv(&mut result)
    ///    .expect("batch receive failed");
    ///assert_eq!(result, expected)
    /// ```
    fn send_batch(&mut self, values: Vec<Self::Item>) -> Result<(), SenderError>;
    /// Async version of [`Sender::send`]
    async fn async_send(&mut self, value: Self::Item) -> Result<(), SenderError>;
    /// Async version of [`Sender::send_batch`]
    async fn async_send_batch(&mut self, values: Vec<Self::Item>) -> Result<(), SenderError>;
}

#[derive(Debug, Clone)]
pub struct BroadcastSender<T> {
    inner: SenderCore<T, BroadcastTracker>,
}

impl<T> From<&BroadcastReceiver<T>> for BroadcastSender<T> {
    fn from(receiver: &BroadcastReceiver<T>) -> Self {
        let core = receiver.clone_core();
        Self { inner: core.into() }
    }
}

impl<T> BroadcastSender<T> {
    pub(crate) fn clone_core(&self) -> Arc<Core<T, BroadcastTracker>> {
        self.inner.disruptor.clone()
    }
    /// Creates a new receiver to the same channel. Points to the most
    /// recently sent value in the channel
    pub fn new_receiver(&self) -> BroadcastReceiver<T> {
        self.into()
    }
}

#[async_trait]
impl<T> Sender for BroadcastSender<T>
where
    T: Send,
{
    type Item = T;

    fn send(&mut self, value: Self::Item) -> Result<(), SenderError> {
        self.inner.send(value)
    }
    fn send_batch(&mut self, values: Vec<Self::Item>) -> Result<(), SenderError> {
        self.inner.send_batch(values)
    }

    async fn async_send(&mut self, value: Self::Item) -> Result<(), SenderError> {
        self.inner.async_send(value).await
    }

    async fn async_send_batch(&mut self, values: Vec<Self::Item>) -> Result<(), SenderError> {
        self.inner.async_send_batch(values).await
    }
}

impl<T> From<SenderCore<T, BroadcastTracker>> for BroadcastSender<T> {
    fn from(sender: SenderCore<T, BroadcastTracker>) -> Self {
        Self { inner: sender }
    }
}

#[derive(Debug)]
pub(crate) struct SenderCore<T, TR> {
    disruptor: Arc<Core<T, TR>>,
    capacity: isize,
    cached_slowest_reader: isize,
}

impl<T, TR> Clone for SenderCore<T, TR> {
    fn clone(&self) -> Self {
        Self {
            disruptor: self.disruptor.clone(),
            capacity: self.capacity,
            cached_slowest_reader: self.cached_slowest_reader,
        }
    }
}

impl<T, TR> From<Arc<Core<T, TR>>> for SenderCore<T, TR> {
    fn from(disruptor: Arc<Core<T, TR>>) -> Self {
        let capacity = disruptor.capacity;
        Self {
            disruptor,
            capacity,
            cached_slowest_reader: -1,
        }
    }
}

impl<T, TR> SenderCore<T, TR>
where
    T: Send,
    TR: Tracker,
{
    //TODO this can probably be cut down / optimised a bit...
    fn claim(&mut self, num_to_claim: isize) -> Result<isize, SenderError> {
        if num_to_claim > self.capacity {
            return Err(SenderError::InputTooLarge);
        }
        let claimed = self
            .disruptor
            .claimed
            .fetch_add(num_to_claim, Ordering::Release);
        let tail = claimed - self.capacity;

        if tail < -1 || (self.cached_slowest_reader != -1 && self.cached_slowest_reader > tail) {
            return Ok(claimed);
        }

        let mut slowest_reader = self.disruptor.readers.slowest(tail);

        while slowest_reader <= tail {
            let listener = self.disruptor.reader_move.listen();
            // the reader may have moved before we managed to register the listener so
            // we need to check again before we wait. If a reader moves between now and when
            // we call wait it should return immediately
            slowest_reader = self.disruptor.readers.slowest(tail);
            if slowest_reader > tail {
                break;
            }
            listener.wait();
            slowest_reader = self.disruptor.readers.slowest(tail);
        }
        self.cached_slowest_reader = slowest_reader;
        // TODO check if there is another writer writing to a different ID but the same cell.

        Ok(claimed)
    }

    async fn async_claim(&mut self, num_to_claim: isize) -> Result<isize, SenderError> {
        if num_to_claim > self.capacity {
            return Err(SenderError::InputTooLarge);
        }
        let claimed = self
            .disruptor
            .claimed
            .fetch_add(num_to_claim, Ordering::Release);
        let tail = claimed - self.capacity;

        if self.cached_slowest_reader != -1 && self.cached_slowest_reader > tail {
            return Ok(claimed);
        }

        let mut slowest_reader = self.disruptor.readers.slowest(tail);

        while slowest_reader <= tail {
            let listener = self.disruptor.reader_move.listen();
            // the reader may have moved before we managed to register the listener so
            // we need to check again before we wait. If a reader moves between now and when
            // we call wait it should return immediately
            slowest_reader = self.disruptor.readers.slowest(tail);
            if slowest_reader > tail {
                break;
            }
            listener.await;
            slowest_reader = self.disruptor.readers.slowest(tail);
        }
        self.cached_slowest_reader = slowest_reader;
        // TODO check if there is another writer writing to a different ID but the same cell.

        Ok(claimed)
    }
    pub fn send(&mut self, value: T) -> Result<(), SenderError> {
        let claimed_id = self.claim(1)?;
        self.internal_send(value, claimed_id)
    }
    pub async fn async_send(&mut self, value: T) -> Result<(), SenderError> {
        let claimed_id = self.async_claim(1).await?;

        self.internal_send(value, claimed_id)
    }

    #[inline(always)]
    fn internal_send(&mut self, value: T, claimed_id: isize) -> Result<(), SenderError> {
        let index = claimed_id as usize % self.capacity as usize;

        let old_value;
        unsafe {
            old_value = std::mem::replace((*self.disruptor.ring).get_unchecked_mut(index), value)
        }

        // TODO what's the optimisation here
        // wait for writers to catch up and commit their transactions
        while self
            .disruptor
            .committed
            .compare_exchange_weak(
                claimed_id - 1,
                claimed_id,
                Ordering::Release,
                Ordering::Relaxed,
            )
            .is_err()
        {}
        // Notify other threads that a value has been written
        self.disruptor.writer_move.notify(usize::MAX);

        // We do this at the end to ensure that we're not worrying about wierd drop functions or
        // allocations happening during the critical path
        if (claimed_id as usize) < self.capacity as usize {
            forget(old_value);
        } else {
            drop(old_value);
        }
        Ok(())
    }

    #[inline(always)]
    fn internal_send_batch(
        &mut self,
        mut data: Vec<T>,
        start_id: isize,
    ) -> Result<(), SenderError> {
        let total_num = data.len() as isize;
        let start_index = (start_id % self.capacity) as usize;
        let end_index = start_index + data.len();

        if end_index < self.capacity as usize {
            unsafe {
                let target = &mut (*self.disruptor.ring)[start_index..end_index];
                std::ptr::swap_nonoverlapping(data.as_mut_ptr(), target.as_mut_ptr(), data.len());
            }
            if start_id < self.capacity {
                forget(data.into_boxed_slice());
            }
        } else {
            //TODO surely there's a better way
            let mut index = start_index;
            for value in data {
                if index < self.capacity as usize {
                    //forget the old value
                    unsafe {
                        let old = std::mem::replace(
                            &mut (*self.disruptor.ring)[index % self.capacity as usize],
                            value,
                        );
                        forget(old);
                    }
                } else {
                    //drop the old value
                    unsafe {
                        (*self.disruptor.ring)[index % self.capacity as usize] = value;
                    }
                }
                index += 1;
            }
        }

        while self
            .disruptor
            .committed
            .compare_exchange_weak(
                start_id - 1,
                start_id + total_num - 1,
                Ordering::Release,
                Ordering::Relaxed,
            )
            .is_err()
        {}
        // Notify other threads that a value has been written
        self.disruptor.writer_move.notify(usize::MAX);

        Ok(())
    }

    pub fn send_batch(&mut self, data: Vec<T>) -> Result<(), SenderError> {
        let start_id = self.claim(data.len() as isize)?;

        self.internal_send_batch(data, start_id)
    }

    pub async fn async_send_batch(&mut self, data: Vec<T>) -> Result<(), SenderError> {
        let num_to_claim = data.len() as isize;
        let start_id = self.async_claim(num_to_claim).await?;
        self.internal_send_batch(data, start_id)
    }
}

#[cfg(test)]
mod sender_tests {
    use crate::channel::sender::Sender;
    use crate::channel::*;
    use crate::{BroadcastSender, Receiver};

    #[test]
    fn batch_write() {
        let (mut sender, mut receiver) = channel(50);
        let expected: Vec<i32> = (0..12).collect();
        sender.send_batch(expected).expect("send failed");
        let mut result = Vec::with_capacity(12);
        receiver
            .batch_recv(&mut result)
            .expect("receiver was okay!");

        let expected: Vec<i32> = (0..12).collect();
        assert_eq!(result, expected)
    }

    #[test]
    fn batch_write_wrapping() {
        let (mut sender, mut receiver) = channel(10);
        sender.send(0).expect("couldn't send");
        sender.send(1).expect("couldn't send");
        let _ = receiver.recv().expect("couldn't receive");
        let _ = receiver.recv().expect("couldn't receive");
        let expected: Vec<i32> = (2..12).collect();
        sender.send_batch(expected).expect("send failed");
        let mut result = Vec::with_capacity(12);
        receiver
            .batch_recv(&mut result)
            .expect("receiver was okay!");

        let expected: Vec<i32> = (2..12).collect();
        assert_eq!(result, expected)
    }

    #[test]
    fn sender_from_receiver() {
        let (_, mut receiver) = channel(10);
        let mut sender: BroadcastSender<i32> = (&receiver).into();
        sender.send(42).expect("couldn't send");
        let v = receiver.recv().expect("couldn't receive");
        assert_eq!(v, 42);
    }
}
