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
    /// Async version of [`Sender::send`]
    async fn async_send(&mut self, value: Self::Item) -> Result<(), SenderError>;
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
    async fn async_send(&mut self, value: Self::Item) -> Result<(), SenderError> {
        self.inner.async_send(value).await
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
    capacity: usize,
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
    fn claim(&mut self, num_to_claim: usize) -> Result<isize, SenderError> {
        if num_to_claim > self.capacity {
            return Err(SenderError::InputTooLarge);
        }
        let claimed = self
            .disruptor
            .claimed
            .fetch_add(num_to_claim as isize, Ordering::SeqCst);
        let tail = claimed - self.capacity as isize;

        if tail < 0 || (self.cached_slowest_reader != -1 && self.cached_slowest_reader > tail) {
            return Ok(claimed);
        }

        self.cached_slowest_reader = self.disruptor.readers.wait_for_tail(tail as usize) as isize;

        Ok(claimed)
    }

    async fn async_claim(&mut self, num_to_claim: usize) -> Result<isize, SenderError> {
        if num_to_claim > self.capacity {
            return Err(SenderError::InputTooLarge);
        }
        let claimed = self
            .disruptor
            .claimed
            .fetch_add(num_to_claim as isize, Ordering::SeqCst);
        let tail = claimed - self.capacity as isize;

        if tail < 0 {
            return Ok(claimed);
        }

        self.cached_slowest_reader = self
            .disruptor
            .readers
            .wait_for_tail_async(tail as usize + 1)
            .await as isize;

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
        let index = claimed_id as usize % self.capacity;

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
                Ordering::SeqCst,
                Ordering::SeqCst,
            )
            .is_err()
        {}
        // Notify other threads that a value has been written
        self.disruptor.writer_move.notify(usize::MAX);

        // We do this at the end to ensure that we're not worrying about wierd drop functions or
        // allocations happening during the critical path
        if (claimed_id as usize) < self.capacity {
            forget(old_value);
        } else {
            drop(old_value);
        }
        Ok(())
    }
}

#[cfg(test)]
mod sender_tests {
    use crate::channel::sender::Sender;
    use crate::channel::*;
    use crate::{BroadcastSender, Receiver};

    #[test]
    fn sender_from_receiver() {
        let (_, mut receiver) = channel(10);
        let mut sender: BroadcastSender<i32> = (&receiver).into();
        sender.send(42).expect("couldn't send");
        let v = receiver.recv().expect("couldn't receive");
        assert_eq!(v, 42);
    }
}
