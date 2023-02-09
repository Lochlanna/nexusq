use super::*;
use crate::channel::tracker::Tracker;
use alloc::sync::Arc;
use core::mem::forget;

#[derive(Debug)]
pub enum SenderError {
    /// The given input is too large to fit in the buffered channel
    InputTooLarge,
    ChannelFull,
}

pub trait Sender<T: Send>: Clone {
    /// Send a single value to the channel. This function will block if there is no space
    /// available in the channel.
    fn send(&mut self, value: T);
}

#[derive(Debug)]
pub struct BroadcastSender<CORE> {
    core: Arc<CORE>,
    capacity: isize,
    cached_slowest_reader: isize,
}

impl<CORE> Clone for BroadcastSender<CORE> {
    fn clone(&self) -> Self {
        Self {
            core: self.core.clone(),
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
        let capacity = disruptor.capacity() as isize;
        let slowest = disruptor.reader_tracker().current();
        Self {
            core: disruptor,
            capacity,
            cached_slowest_reader: slowest,
        }
    }
}

impl<CORE> From<BroadcastReceiver<CORE>> for BroadcastSender<CORE>
where
    CORE: Core,
{
    fn from(receiver: BroadcastReceiver<CORE>) -> Self {
        receiver.get_core().into()
    }
}

impl<CORE> BroadcastSender<CORE>
where
    CORE: Core,
{
    //TODO this can probably be cut down / optimised a bit...
    fn claim(&mut self, num_to_claim: usize) -> Result<isize, SenderError> {
        if num_to_claim as isize > self.capacity {
            return Err(SenderError::InputTooLarge);
        }

        let claimed = self.core.sender_tracker().claim(num_to_claim);
        let tail = claimed - self.capacity;

        if tail >= 0 {
            self.core.reader_tracker().wait_for(tail);
        }

        Ok(claimed)
    }

    pub fn send(&mut self, value: CORE::T) {
        let claimed_id = self.claim(1).expect("this should never fail");
        self.internal_send(value, claimed_id)
    }

    #[inline(always)]
    fn internal_send(&mut self, value: CORE::T, claimed_id: isize) {
        debug_assert!(claimed_id >= 0);
        let index = claimed_id.fmod(self.capacity) as usize;

        let old_value;
        unsafe {
            old_value = core::mem::replace((*self.core.ring()).get_unchecked_mut(index), value)
        }

        // Notify other threads that a value has been written
        self.core.sender_tracker().commit(claimed_id);

        // We do this at the end to ensure that we're not worrying about wierd drop functions or
        // allocations happening during the critical path
        if claimed_id < self.capacity {
            forget(old_value);
        } else {
            drop(old_value);
        }
    }

    pub(crate) fn get_core(&self) -> Arc<CORE> {
        self.core.clone()
    }
}

impl<CORE> Sender<CORE::T> for BroadcastSender<CORE>
where
    CORE: Core,
    <CORE as Core>::T: Send,
{
    fn send(&mut self, value: CORE::T) {
        BroadcastSender::send(self, value)
    }
}

#[cfg(test)]
mod sender_tests {
    use crate::channel::*;

    #[test]
    fn sender_from_receiver() {
        let (_, mut receiver) = channel(10);
        let mut sender: BroadcastSender<Ring<i32, SpinBlockWait, SpinBlockWait>> =
            receiver.clone().into();
        sender.send(42);
        let v = receiver.recv();
        assert_eq!(v, 42);
    }
}
