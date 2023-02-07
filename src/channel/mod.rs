pub mod receiver;
pub mod sender;
pub mod tracker;
pub mod wait_strategy;

use crate::channel::tracker::{
    ProducerTracker, ReceiverTracker, SequentialProducerTracker, Tracker,
};
use crate::{BroadcastReceiver, BroadcastSender};
use alloc::boxed::Box;
use alloc::sync::Arc;
use alloc::vec::Vec;
use tracker::broadcast_tracker::MultiCursorTracker;
use wait_strategy::{BusySpinWaitStrategy, WaitStrategy};

pub trait FastMod: Sized {
    fn fmod(&self, denominator: Self) -> Self;
}
impl FastMod for isize {
    fn fmod(&self, denominator: Self) -> Self {
        debug_assert!(*self >= 0);
        debug_assert!(denominator.is_positive());
        // is pow 2 https://graphics.stanford.edu/~seander/bithacks.html#DetermineIfPowerOf2
        debug_assert!((denominator & (denominator - 1)) == 0);
        *self & (denominator - 1)
    }
}

impl FastMod for usize {
    fn fmod(&self, denominator: Self) -> Self {
        // is pow 2 https://graphics.stanford.edu/~seander/bithacks.html#DetermineIfPowerOf2
        debug_assert!((denominator & (denominator - 1)) == 0);
        *self & (denominator - 1)
    }
}

pub trait Core {
    type T;
    type SendTracker: Tracker + ProducerTracker;
    type ReadTracker: Tracker + ReceiverTracker;
    fn sender_tracker(&self) -> &Self::SendTracker;
    fn reader_tracker(&self) -> &Self::ReadTracker;
    fn ring(&self) -> *mut Vec<Self::T>;
    fn capacity(&self) -> usize;
}

#[derive(Debug)]
pub struct Ring<T, WWS, RWS> {
    ring: *mut Vec<T>,
    capacity: usize,
    // is there a better way than events?
    sender_tracker: SequentialProducerTracker<WWS>,
    // Reference to each reader to get their position. It should be sorted(how..?)
    reader_tracker: MultiCursorTracker<RWS>,
}

unsafe impl<T, WWS, RWS> Send for Ring<T, WWS, RWS> {}
unsafe impl<T, WWS, RWS> Sync for Ring<T, WWS, RWS> {}

impl<T, WWS, RWS> Drop for Ring<T, WWS, RWS> {
    fn drop(&mut self) {
        unsafe {
            drop(Box::from_raw(self.ring));
        }
    }
}

impl<T, WWS, RWS> Ring<T, WWS, RWS>
where
    WWS: WaitStrategy,
    RWS: WaitStrategy,
{
    pub(crate) fn new(
        mut buffer_size: usize,
        write_wait_strategy: WWS,
        read_wait_strategy: RWS,
    ) -> Self {
        buffer_size = buffer_size
            .checked_next_power_of_two()
            .expect("usize wrapped!");
        let mut ring = Box::new(Vec::with_capacity(buffer_size));
        let capacity = ring.capacity();
        //TODO check that it's not bigger than isize::MAX
        unsafe {
            // use capacity as vec is allowed to allocate more than buffer_size if it likes so
            //we might as well use it!
            ring.set_len(capacity);
        }

        let ring = Box::into_raw(ring);

        Self {
            ring,
            capacity,
            sender_tracker: SequentialProducerTracker::new(write_wait_strategy),
            reader_tracker: MultiCursorTracker::new(buffer_size, read_wait_strategy),
        }
    }
}

impl<T, WWS, RWS> Core for Ring<T, WWS, RWS>
where
    WWS: WaitStrategy,
    RWS: WaitStrategy,
{
    type T = T;
    type SendTracker = SequentialProducerTracker<WWS>;
    type ReadTracker = MultiCursorTracker<RWS>;

    fn sender_tracker(&self) -> &Self::SendTracker {
        &self.sender_tracker
    }

    fn reader_tracker(&self) -> &Self::ReadTracker {
        &self.reader_tracker
    }

    fn ring(&self) -> *mut Vec<Self::T> {
        self.ring
    }

    fn capacity(&self) -> usize {
        self.capacity
    }
}

///Creates a new mpmc broadcast channel returning both a sender and receiver
pub fn channel<T>(
    size: usize,
) -> (
    BroadcastSender<Ring<T, BusySpinWaitStrategy, BusySpinWaitStrategy>>,
    BroadcastReceiver<Ring<T, BusySpinWaitStrategy, BusySpinWaitStrategy>>,
) {
    let ws = BusySpinWaitStrategy::default();
    let core = Arc::new(Ring::<T, _, _>::new(size, ws.clone(), ws));
    let sender = sender::BroadcastSender::from(core.clone());
    let receiver = receiver::BroadcastReceiver::from(core);
    (sender, receiver)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channel::sender::Sender;
    use crate::Receiver;
    use std::prelude::v1::String;
    use std::thread::{spawn, JoinHandle};

    #[inline(always)]
    fn read_n(mut receiver: impl Receiver<usize>, num_to_read: usize) -> Vec<usize> {
        let mut results = Vec::with_capacity(num_to_read);
        for _ in 0..num_to_read {
            let v = receiver.recv();
            assert!(v.is_ok());
            results.push(v.unwrap());
        }
        results
    }

    #[inline(always)]
    fn write_n(mut sender: impl Sender<usize>, num_to_write: usize) {
        for i in 0..num_to_write {
            sender.send(i).expect("couldn't send");
        }
    }

    #[inline(always)]
    fn test(num_elements: usize, num_writers: usize, num_readers: usize, buffer_size: usize) {
        let (sender, receiver) = channel(buffer_size);
        let readers: Vec<JoinHandle<Vec<usize>>> = (0..num_readers)
            .map(|_| {
                let new_receiver = receiver.clone();
                spawn(move || read_n(new_receiver, num_elements * num_writers))
            })
            .collect();
        drop(receiver);
        let writers: Vec<JoinHandle<()>> = (0..num_writers)
            .map(|_| {
                let new_sender = sender.clone();
                spawn(move || {
                    write_n(new_sender, num_elements);
                })
            })
            .collect();

        for writer in writers {
            let _ = writer.join();
        }
        for reader in readers {
            let res = reader.join();
            match res {
                Ok(res) => {
                    assert_eq!(res.len(), num_elements * num_writers);
                    if num_writers == 1 {
                        let expected: Vec<usize> = (0..num_elements).collect();
                        assert_eq!(res, expected);
                    }
                }
                Err(_) => panic!("reader didnt' read enough"),
            }
        }
    }

    #[test]
    fn single_writer_single_reader() {
        let num = 5000;
        test(num, 1, 1, 10);
    }

    #[test]
    fn single_writer_single_reader_clone() {
        let (mut sender, mut receiver) = channel(10);
        sender
            .send(String::from("hello world"))
            .expect("couldn't send");
        let res = receiver.recv();
        assert_eq!(res, "hello world");
    }

    #[test]
    fn single_writer_two_reader() {
        let num = 5000;
        test(num, 1, 2, 10);
    }

    #[test]
    fn two_writer_two_reader() {
        let num = 5000;
        test(num, 2, 2, 10);
    }

    #[test]
    fn three_writer_three_reader() {
        let num = 5000;
        test(num, 3, 3, 10);
    }
}
