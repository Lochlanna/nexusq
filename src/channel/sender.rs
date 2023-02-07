use super::*;
use crate::channel::tracker::broadcast_tracker::MultiCursorTracker;
use crate::channel::tracker::Tracker;
use alloc::sync::Arc;
use async_trait::async_trait;
use core::mem::forget;
use core::sync::atomic::Ordering;

#[derive(Debug)]
pub enum SenderError {
    /// The given input is too large to fit in the buffered channel
    InputTooLarge,
}

#[async_trait]
pub trait Sender<T: Send>: Clone {
    /// Send a single value to the channel. This function will block if there is no space
    /// available in the channel.
    fn send(&mut self, value: T) -> Result<(), SenderError>;
}

#[derive(Debug)]
pub struct BroadcastSender<CORE> {
    disruptor: Arc<CORE>,
    capacity: usize,
    cached_slowest_reader: isize,
}

impl<CORE> Clone for BroadcastSender<CORE> {
    fn clone(&self) -> Self {
        Self {
            disruptor: self.disruptor.clone(),
            capacity: self.capacity,
            cached_slowest_reader: self.cached_slowest_reader,
        }
    }
}

impl<CORE> From<Arc<CORE>> for BroadcastSender<CORE>
where
    CORE: Core,
{
    fn from(disruptor: Arc<CORE>) -> Self {
        let capacity = disruptor.capacity();
        let slowest = disruptor.reader_tracker().current();
        Self {
            disruptor,
            capacity,
            cached_slowest_reader: slowest,
        }
    }
}

impl<CORE> BroadcastSender<CORE>
where
    CORE: Core,
{
    //TODO this can probably be cut down / optimised a bit...
    fn claim(&mut self, num_to_claim: usize) -> Result<isize, SenderError> {
        if num_to_claim > self.capacity {
            return Err(SenderError::InputTooLarge);
        }

        let claimed = self.disruptor.sender_tracker().claim(num_to_claim);
        let tail = claimed - self.capacity as isize;

        if tail < 0 || self.cached_slowest_reader > tail {
            return Ok(claimed);
        }

        self.cached_slowest_reader = self.disruptor.reader_tracker().wait_for(tail + 1) as isize;

        Ok(claimed)
    }

    pub fn send(&mut self, value: CORE::T) -> Result<(), SenderError> {
        let claimed_id = self.claim(1)?;
        self.internal_send(value, claimed_id)
    }

    #[inline(always)]
    fn internal_send(&mut self, value: CORE::T, claimed_id: isize) -> Result<(), SenderError> {
        let index = claimed_id as usize % self.capacity;

        let old_value;
        unsafe {
            old_value = core::mem::replace((*self.disruptor.ring()).get_unchecked_mut(index), value)
        }

        // Notify other threads that a value has been written
        self.disruptor.sender_tracker().commit(claimed_id);

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

impl<CORE> Sender<CORE::T> for BroadcastSender<CORE>
where
    CORE: Core,
    <CORE as Core>::T: Send,
{
    fn send(&mut self, value: CORE::T) -> Result<(), SenderError> {
        BroadcastSender::send(self, value)
    }
}

#[cfg(test)]
mod sender_tests {
    use crate::channel::sender::Sender;
    use crate::channel::*;
    use crate::Receiver;

    #[test]
    fn sender_from_receiver() {
        // let (_, mut receiver) = channel(10);
        // let mut sender: BroadcastSender<i32> = (&receiver).into();
        // sender.send(42).expect("couldn't send");
        // let v = receiver.recv().expect("couldn't receive");
        // assert_eq!(v, 42);
    }
}
