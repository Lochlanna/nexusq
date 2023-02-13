pub mod receiver;
pub mod sender;
pub mod tracker;
pub mod wait_strategy;

use crate::channel::tracker::{ProducerTracker, ReceiverTracker, Tracker};
use crate::channel::wait_strategy::{BusyWait, SpinBlockWait};
use crate::{BroadcastReceiver, BroadcastSender};
use alloc::boxed::Box;
use alloc::sync::Arc;
use alloc::vec::Vec;
use thiserror::Error as ThisError;
use tracker::MultiCursorTracker;
use tracker::SequentialProducerTracker;
use wait_strategy::WaitStrategy;

#[derive(Debug, ThisError)]
pub enum Error {
    #[error("size must be a power of 2")]
    InvalidSize,
    #[error("requested buffer too big")]
    BufferTooBig,
}

impl From<tracker::Error> for Error {
    fn from(error: tracker::Error) -> Self {
        match error {
            tracker::Error::InvalidSize => Self::InvalidSize,
        }
    }
}

pub trait FastMod: Sized {
    fn pow_2_mod(&self, denominator: Self) -> Self;
}
impl FastMod for isize {
    #[inline(always)]
    fn pow_2_mod(&self, denominator: Self) -> Self {
        debug_assert!(*self >= 0);
        debug_assert!(denominator.is_positive());
        debug_assert!((denominator as usize).is_power_of_two());
        *self & (denominator - 1)
    }
}

impl FastMod for usize {
    #[inline(always)]
    fn pow_2_mod(&self, denominator: Self) -> Self {
        debug_assert!(denominator.is_power_of_two());
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
pub struct Ring<T, WTWS, RTWS> {
    ring: *mut Vec<T>,
    capacity: usize,
    // is there a better way than events?
    sender_tracker: SequentialProducerTracker<WTWS>,
    // Reference to each reader to get their position. It should be sorted(how..?)
    reader_tracker: MultiCursorTracker<RTWS>,
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
        write_tracker_wait_strategy: WWS,
        read_tracker_wait_strategy: RWS,
    ) -> Result<Self, Error> {
        buffer_size = if let Some(bs) = buffer_size.checked_next_power_of_two() {
            bs
        } else {
            return Err(Error::BufferTooBig);
        };
        if buffer_size > isize::MAX as usize {
            return Err(Error::BufferTooBig);
        }
        let mut ring = Box::new(Vec::with_capacity(buffer_size));
        unsafe {
            ring.set_len(buffer_size);
        }

        let ring = Box::into_raw(ring);

        Ok(Self {
            ring,
            capacity: buffer_size,
            sender_tracker: SequentialProducerTracker::new(write_tracker_wait_strategy),
            reader_tracker: MultiCursorTracker::new(buffer_size, read_tracker_wait_strategy)?,
        })
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

pub struct ChannelHandles<T, WTWS: WaitStrategy, RTWS: WaitStrategy> {
    pub sender: BroadcastSender<Ring<T, WTWS, RTWS>>,
    pub receiver: BroadcastReceiver<Ring<T, WTWS, RTWS>>,
}

impl<T, WTWS, RTWS> ChannelHandles<T, WTWS, RTWS>
where
    WTWS: WaitStrategy,
    RTWS: WaitStrategy,
{
    fn new(
        sender: BroadcastSender<Ring<T, WTWS, RTWS>>,
        receiver: BroadcastReceiver<Ring<T, WTWS, RTWS>>,
    ) -> Self {
        Self { sender, receiver }
    }
    //Not sure how to get around the complexity without using dyn
    #[allow(clippy::type_complexity)]
    pub fn dissolve(
        self,
    ) -> (
        BroadcastSender<Ring<T, WTWS, RTWS>>,
        BroadcastReceiver<Ring<T, WTWS, RTWS>>,
    ) {
        (self.sender, self.receiver)
    }
}

///Creates a new mpmc broadcast channel returning both a sender and receiver
pub fn channel<T>(size: usize) -> Result<ChannelHandles<T, SpinBlockWait, SpinBlockWait>, Error> {
    channel_with(size, SpinBlockWait::default(), SpinBlockWait::default())
}

pub fn busy_channel<T>(size: usize) -> Result<ChannelHandles<T, BusyWait, BusyWait>, Error> {
    channel_with(size, Default::default(), Default::default())
}

pub fn channel_with<T, WTWS, RTWS>(
    size: usize,
    write_tracker_wait_strategy: WTWS,
    read_tracker_wait_strategy: RTWS,
) -> Result<ChannelHandles<T, WTWS, RTWS>, Error>
where
    WTWS: WaitStrategy,
    RTWS: WaitStrategy,
{
    let core = Arc::new(Ring::<T, _, _>::new(
        size,
        write_tracker_wait_strategy,
        read_tracker_wait_strategy,
    )?);
    let sender = sender::BroadcastSender::from(core.clone());
    let receiver = receiver::BroadcastReceiver::from(core);
    Ok(ChannelHandles::new(sender, receiver))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channel::sender::Sender;
    use crate::Receiver;
    use pretty_assertions::assert_eq;
    use std::collections::HashMap;
    use std::thread::{spawn, JoinHandle};
    use std::time::Duration;

    #[inline(always)]
    fn read_n(
        mut receiver: impl Receiver<usize>,
        num_to_read: usize,
        sleep_time: Duration,
        thread_num: usize,
    ) -> Vec<usize> {
        let mut results = Vec::with_capacity(num_to_read);
        let seed = 42 + thread_num;
        let jtter_duration = sleep_time + sleep_time.div_f32(0.5);
        for i in 0..num_to_read {
            let v = receiver.recv();
            assert!(v.is_ok());
            jitter_sleep(sleep_time, seed, jtter_duration, i);
            results.push(v.unwrap());
        }
        results
    }

    fn jitter_sleep(sleep_time: Duration, seed: usize, jtter_duration: Duration, i: usize) {
        if !sleep_time.is_zero() {
            if i % seed == 0 {
                std::thread::sleep(jtter_duration);
            } else {
                std::thread::sleep(sleep_time);
            }
        }
    }

    #[inline(always)]
    fn write_n(
        mut sender: impl Sender<usize>,
        num_to_write: usize,
        sleep_time: Duration,
        thread_num: usize,
    ) {
        let seed = 42 + thread_num;
        let jtter_duration = sleep_time + sleep_time.div_f32(0.5);
        for i in 0..num_to_write {
            sender.send(i);
            jitter_sleep(sleep_time, seed, jtter_duration, i);
        }
    }

    #[inline(always)]
    fn test(
        num_elements: usize,
        num_writers: usize,
        num_readers: usize,
        buffer_size: usize,
        read_sleep: Duration,
        write_sleep: Duration,
    ) {
        let ChannelHandles { sender, receiver } =
            channel(buffer_size).expect("couldn't create channel");
        let readers: Vec<JoinHandle<Vec<usize>>> = (0..num_readers)
            .map(|i| {
                let new_receiver = receiver.clone();
                spawn(move || read_n(new_receiver, num_elements * num_writers, read_sleep, i))
            })
            .collect();
        drop(receiver);
        let writers: Vec<JoinHandle<()>> = (0..num_writers)
            .map(|i| {
                let new_sender = sender.clone();
                spawn(move || {
                    write_n(new_sender, num_elements, write_sleep, i);
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
                    let mut expected = HashMap::with_capacity(num_elements);
                    (0..num_elements).for_each(|v| {
                        expected.insert(v, num_writers);
                    });
                    let mut resmap = HashMap::with_capacity(num_elements);
                    res.into_iter().for_each(|v| {
                        let e = resmap.entry(v).or_insert(0_usize);
                        *e += 1;
                    });
                    let mut missing = HashMap::new();
                    for (key, value) in &expected {
                        if let Some(val) = resmap.get(key) {
                            if val != value {
                                missing.insert(*key, *val);
                            }
                            continue;
                        }
                        missing.insert(*key, 0);
                    }
                    if !missing.is_empty() {
                        println!("diff is {missing:?}");
                    }
                    assert_eq!(resmap, expected);
                }
                Err(_) => panic!("reader didnt' read enough"),
            }
        }
    }

    #[test]
    fn single_writer_single_reader() {
        let num = 15;
        test(num, 1, 1, 5, Default::default(), Default::default());
    }

    #[test]
    fn single_writer_single_reader_clone() {
        let ChannelHandles {
            mut sender,
            mut receiver,
        } = channel(10).expect("couldn't create channel");
        sender.send(String::from("hello world"));
        let res = receiver.recv();
        assert_eq!(res, "hello world");
    }

    #[test]
    fn single_writer_two_reader() {
        let num = 5000;
        test(num, 1, 2, 10, Default::default(), Default::default());
    }

    #[test]
    fn two_writer_two_reader() {
        let num = 5000;
        test(num, 2, 2, 10, Default::default(), Default::default());
    }

    #[test]
    fn two_writer_two_reader_slow_read() {
        let num = 500;
        test(num, 2, 2, 4, Duration::from_millis(1), Default::default());
    }

    #[test]
    fn two_writer_two_reader_slow_write() {
        let num = 500;
        test(num, 2, 2, 4, Default::default(), Duration::from_millis(1));
    }

    #[test]
    fn two_writer_two_reader_very_slow_read() {
        let num = 100;
        test(num, 2, 2, 4, Duration::from_millis(2), Default::default());
    }

    #[test]
    fn two_writer_two_reader_very_slow_write() {
        let num = 100;
        test(num, 2, 2, 4, Default::default(), Duration::from_millis(2));
    }

    #[test]
    fn two_writer_two_reader_slow_read_write() {
        let num = 200;
        test(
            num,
            2,
            2,
            4,
            Duration::from_micros(20),
            Duration::from_micros(20),
        );
    }

    #[test]
    fn three_writer_three_reader() {
        let num = 5000;
        test(num, 3, 3, 10, Default::default(), Default::default());
    }

    #[test]
    #[ignore]
    fn ten_writer_ten_reader() {
        let num = 2000;
        test(num, 10, 10, 10, Default::default(), Default::default());
    }

    #[test]
    #[ignore]
    fn ten_writer_ten_reader_slow_read() {
        let num = 200;
        test(
            num,
            10,
            10,
            10,
            Duration::from_micros(20),
            Default::default(),
        );
    }
    #[test]
    fn ten_writer_ten_reader_slow_write() {
        let num = 200;
        test(
            num,
            10,
            10,
            10,
            Default::default(),
            Duration::from_micros(20),
        );
    }

    #[test]
    #[ignore]
    fn ten_writer_ten_reader_slow_read_write() {
        let num = 200;
        test(
            num,
            10,
            10,
            10,
            Duration::from_micros(20),
            Duration::from_micros(20),
        );
    }
}
