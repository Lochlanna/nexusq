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
    disruptor: Arc<CORE>,
    internal_cursor: isize,
    capacity: isize,
    committed_cache: isize,
}

impl<CORE> Drop for BroadcastReceiver<CORE>
where
    CORE: Core,
{
    fn drop(&mut self) {
        self.disruptor
            .reader_tracker()
            .de_register(self.internal_cursor);
    }
}

impl<CORE> From<Arc<CORE>> for BroadcastReceiver<CORE>
where
    CORE: Core,
{
    fn from(disruptor: Arc<CORE>) -> Self {
        let internal_cursor = disruptor.reader_tracker().register();
        let capacity = disruptor.capacity() as isize;
        let committed = disruptor.sender_tracker().current();
        Self {
            disruptor,
            internal_cursor,
            capacity,
            committed_cache: committed,
        }
    }
}

impl<CORE> Clone for BroadcastReceiver<CORE>
where
    CORE: Core,
{
    /// Creates a new receiver at the same point in the stream
    fn clone(&self) -> Self {
        let tail = self.disruptor.reader_tracker().register();
        self.disruptor
            .reader_tracker()
            .update(tail, self.internal_cursor);
        Self {
            disruptor: self.disruptor.clone(),
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
    fn increment_cursor(&mut self) {
        self.internal_cursor += 1;
        self.disruptor
            .reader_tracker()
            .update(self.internal_cursor - 1, self.internal_cursor)
    }
    /// Creates a new receiver at the most recent entry in the stream
    pub fn add_stream(&self) -> Self {
        self.disruptor.clone().into()
    }
}

impl<CORE> BroadcastReceiver<CORE>
where
    CORE: Core,
    <CORE as Core>::T: Clone,
{
    /// Try to read the next value from the channel. This function will not block and will return
    /// a [`ReaderError::NoNewData`] if there is no data available
    fn try_read_next(&mut self) -> Result<CORE::T, ReaderError> {
        if self.internal_cursor as isize > self.committed_cache {
            self.committed_cache = self.disruptor.sender_tracker().current();
            if self.internal_cursor as isize > self.committed_cache {
                return Err(ReaderError::NoNewData);
            }
        }
        if self.committed_cache < 0 {
            return Err(ReaderError::NoNewData);
        }
        let index = (self.internal_cursor % self.capacity) as usize;
        // the value has been committed so it's safe to read it!
        let value;
        unsafe {
            value = (*self.disruptor.ring()).get_unchecked(index).clone();
        }
        self.increment_cursor();
        Ok(value)
    }

    /// Read the next value from the channel. This function will block and wait for data to
    /// become available.
    pub fn recv(&mut self) -> CORE::T {
        self.disruptor
            .sender_tracker()
            .wait_for(self.internal_cursor as isize);
        self.try_read_next().expect("value wasn't ready!")
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
    use crate::channel::sender::Sender;
    use crate::channel::*;
    use crate::wait_strategy::BusySpinWaitStrategy;
    use crate::Receiver;

    #[test]
    fn receiver_from_sender() {
        // let (mut sender, _) = channel(10);
        // sender.send(42).expect("couldn't send");
        // let mut receiver: BroadcastReceiver<i32, BusySpinWaitStrategy> = (&sender).into();
        // let v = receiver.recv().expect("couldn't receive");
        // assert_eq!(v, 42);
    }
}
