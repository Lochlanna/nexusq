use alloc::sync::Arc;
use core::mem::forget;

use super::tracker::{ProducerTracker, Tracker};
use super::Core;
use crate::channel::Ring;
use crate::utils::FastMod;
use crate::BroadcastReceiver;

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
pub struct BroadcastSender<T> {
    core: Arc<Ring<T>>,
    capacity: isize,
    cached_tail: isize,
}

impl<T> Clone for BroadcastSender<T> {
    fn clone(&self) -> Self {
        Self {
            core: self.core.clone(),
            capacity: self.capacity,
            cached_tail: 0,
        }
    }
}

impl<T> From<Arc<Ring<T>>> for BroadcastSender<T> {
    fn from(disruptor: Arc<Ring<T>>) -> Self {
        let capacity = disruptor.capacity() as isize;
        Self {
            core: disruptor,
            capacity,
            cached_tail: 0,
        }
    }
}

impl<T> From<BroadcastReceiver<T>> for BroadcastSender<T> {
    fn from(receiver: BroadcastReceiver<T>) -> Self {
        receiver.get_core().into()
    }
}

impl<T> BroadcastSender<T> {
    fn claim(&mut self) -> isize {
        let claimed = self.core.sender_tracker().make_claim();

        let tail = claimed - self.capacity;
        if tail >= 0 && self.cached_tail <= tail {
            self.cached_tail = self.core.reader_tracker().wait_for(tail + 1);
        }
        debug_assert!(tail < 0 || self.cached_tail > tail);

        claimed
    }

    pub fn send(&mut self, value: T) {
        let claimed_id = self.claim();
        self.internal_send(value, claimed_id)
    }

    #[inline(always)]
    fn internal_send(&mut self, value: T, claimed_id: isize) {
        debug_assert!(claimed_id >= 0);
        let index = claimed_id.pow_2_mod(self.capacity) as usize;

        let old_value;
        unsafe {
            old_value = core::mem::replace((*self.core.ring()).get_unchecked_mut(index), value)
        }

        // Notify other threads that a value has been written
        self.core.sender_tracker().publish(claimed_id);

        // We do this at the end to ensure that we're not worrying about wierd drop functions or
        // allocations happening during the critical path
        if claimed_id < self.capacity {
            forget(old_value);
        } else {
            drop(old_value);
        }
    }

    pub(crate) fn get_core(&self) -> Arc<Ring<T>> {
        self.core.clone()
    }
}

impl<T> Sender<T> for BroadcastSender<T>
where
    T: Send,
{
    fn send(&mut self, value: T) {
        BroadcastSender::send(self, value)
    }
}

#[cfg(test)]
mod sender_tests {
    use crate::*;

    #[test]
    fn sender_from_receiver() {
        let (_, mut receiver) = channel(10).expect("couldn't create channel").dissolve();
        let mut sender: BroadcastSender<i32> = receiver.clone().into();
        sender.send(42);
        let v = receiver.recv();
        assert_eq!(v, 42);
    }
}
