use super::*;
use crate::channel::tracker::Tracker;
use alloc::sync::Arc;

#[derive(Debug)]
pub enum ReaderError {
    /// There is nothing new to be read from the channel
    NoNewData,
}

pub trait Receiver<T>: Clone {
    fn recv(&mut self) -> Result<T, ReaderError>;
}

#[derive(Debug)]
pub struct BroadcastReceiver<CORE: Core> {
    core: Arc<CORE>,
    internal_cursor: isize,
    capacity: isize,
    committed_cache: isize,
}

impl<CORE> Drop for BroadcastReceiver<CORE>
where
    CORE: Core,
{
    fn drop(&mut self) {
        self.core
            .reader_tracker()
            .de_register(self.internal_cursor.clamp(0, isize::MAX));
    }
}

impl<CORE> From<Arc<CORE>> for BroadcastReceiver<CORE>
where
    CORE: Core,
{
    fn from(core: Arc<CORE>) -> Self {
        //TODO we could have a race condition here where the writer overwrites the committed value
        // before registration happens! Unlikely unless the size is very small and writers are very very fast
        let committed = core.sender_tracker().current();
        let internal_cursor = committed.clamp(0, isize::MAX) - 1;
        core.reader_tracker()
            .register(committed.clamp(0, isize::MAX));

        let capacity = core.capacity() as isize;
        Self {
            core,
            internal_cursor,
            capacity,
            committed_cache: committed,
        }
    }
}

impl<CORE> From<BroadcastSender<CORE>> for BroadcastReceiver<CORE>
where
    CORE: Core,
{
    fn from(sender: BroadcastSender<CORE>) -> Self {
        sender.get_core().into()
    }
}

impl<CORE> Clone for BroadcastReceiver<CORE>
where
    CORE: Core,
{
    /// Creates a new receiver at the same point in the stream
    fn clone(&self) -> Self {
        self.core.reader_tracker().register(self.internal_cursor);
        Self {
            core: self.core.clone(),
            internal_cursor: self.internal_cursor,
            capacity: self.capacity,
            committed_cache: self.committed_cache,
        }
    }
}

impl<CORE> BroadcastReceiver<CORE>
where
    CORE: Core,
{
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
    #[inline(always)]
    fn register(&mut self) {
        //TODO possible race on small buffer fast producer
        let committed = self.core.sender_tracker().current();
        debug_assert!(committed >= -1);
        if committed >= 0 {
            self.internal_cursor = committed - 1;
        }
        self.core
            .reader_tracker()
            .register(self.internal_cursor.clamp(0, isize::MAX));
    }
    /// Creates a new receiver at the most recent entry in the stream
    pub fn add_stream(&self) -> Self {
        self.core.clone().into()
    }
    pub(crate) fn get_core(&self) -> Arc<CORE> {
        self.core.clone()
    }
}

impl<CORE> BroadcastReceiver<CORE>
where
    CORE: Core,
    <CORE as Core>::T: Clone,
{
    /// Read the next value from the channel. This function will block and wait for data to
    /// become available.
    pub fn recv(&mut self) -> CORE::T {
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

impl<CORE> Receiver<CORE::T> for BroadcastReceiver<CORE>
where
    CORE: Core,
    <CORE as Core>::T: Clone,
{
    fn recv(&mut self) -> Result<CORE::T, ReaderError> {
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
        let mut receiver: BroadcastReceiver<Ring<i32, SpinBlockWait, SpinBlockWait>> =
            sender.into();
        let v = receiver.recv();
        assert_eq!(v, 42);
    }
}
