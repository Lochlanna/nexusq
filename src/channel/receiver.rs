use thiserror::Error as ThisError;

use alloc::sync::Arc;

use super::tracker::{ReceiverTracker, Tracker, TrackerError};
use super::Core;
use crate::channel::Ring;
use crate::utils::FastMod;
use crate::BroadcastSender;

#[derive(Debug, ThisError)]
pub enum ReceiverError {
    #[error("There is nothing new to be read from the channel")]
    NoNewData,
    #[error("failed to register the receiver on the channel. Generally a result of the channel being entirely overwritten too quickly")]
    RegistrationFailed(#[from] TrackerError),
}

pub trait Receiver<T>: Clone {
    fn recv(&mut self) -> Result<T, ReceiverError>;
}

#[derive(Debug)]
pub struct BroadcastReceiver<T> {
    core: Arc<Ring<T>>,
    internal_cursor: isize,
    capacity: isize,
    committed_cache: isize,
}

impl<T> Drop for BroadcastReceiver<T> {
    fn drop(&mut self) {
        self.core
            .reader_tracker()
            .de_register(self.internal_cursor.clamp(0, isize::MAX));
    }
}

impl<T> TryFrom<Arc<Ring<T>>> for BroadcastReceiver<T> {
    type Error = ReceiverError;

    fn try_from(core: Arc<Ring<T>>) -> Result<Self, Self::Error> {
        let committed = core.sender_tracker().current();
        let internal_cursor = committed.clamp(0, isize::MAX) - 1;
        core.reader_tracker()
            .register(committed.clamp(0, isize::MAX))?;

        let capacity = core.capacity() as isize;
        Ok(Self {
            core,
            internal_cursor,
            capacity,
            committed_cache: committed,
        })
    }
}

impl<T> TryFrom<BroadcastSender<T>> for BroadcastReceiver<T> {
    type Error = ReceiverError;

    fn try_from(sender: BroadcastSender<T>) -> Result<Self, Self::Error> {
        sender.get_core().try_into()
    }
}

impl<T> Clone for BroadcastReceiver<T> {
    /// Creates a new receiver at the same point in the stream
    fn clone(&self) -> Self {
        self.core
            .reader_tracker()
            .register(self.internal_cursor)
            .expect("couldn't register receiver during clone");
        Self {
            core: self.core.clone(),
            internal_cursor: self.internal_cursor,
            capacity: self.capacity,
            committed_cache: self.committed_cache,
        }
    }
}

impl<T> BroadcastReceiver<T> {
    #[inline(always)]
    fn increment_internal(&mut self) {
        self.internal_cursor += 1;
    }
    #[inline(always)]
    fn publish_position(&self) {
        self.core
            .reader_tracker()
            .update(self.internal_cursor - 1, self.internal_cursor)
    }
    /// Creates a new receiver at the most recent entry in the stream
    pub fn add_stream(&self) -> Result<Self, ReceiverError> {
        self.core.clone().try_into()
    }
    pub(crate) fn get_core(&self) -> Arc<Ring<T>> {
        self.core.clone()
    }
}

impl<T> BroadcastReceiver<T>
where
    T: Clone,
{
    /// Read the next value from the channel. This function will block and wait for data to
    /// become available.
    pub fn recv(&mut self) -> T {
        self.increment_internal();
        if self.committed_cache < self.internal_cursor {
            self.committed_cache = self.core.sender_tracker().wait_for(self.internal_cursor);
        }
        if self.internal_cursor > 0 {
            self.publish_position();
        }
        debug_assert!(self.committed_cache >= self.internal_cursor);
        let index = self.internal_cursor.pow_2_mod(self.capacity) as usize;
        // the value has been committed so it's safe to read it!
        let value;
        unsafe {
            value = (*self.core.ring()).get_unchecked(index).clone();
        }
        value
    }
}

impl<T> Receiver<T> for BroadcastReceiver<T>
where
    T: Clone,
{
    fn recv(&mut self) -> Result<T, ReceiverError> {
        Ok(BroadcastReceiver::recv(self))
    }
}

#[cfg(test)]
mod receiver_tests {
    use crate::channel::*;

    #[test]
    fn receiver_from_sender() {
        let (mut sender, _) = channel(10).expect("couldn't create channel").dissolve();
        sender.send(42);
        let mut receiver: BroadcastReceiver<i32> = sender
            .try_into()
            .expect("couldn't create receiver from sender");
        let v = receiver.recv();
        assert_eq!(v, 42);
    }
}
