pub mod receiver;
pub mod sender;
mod tracker;
pub mod wait_strategy;

use thiserror::Error as ThisError;

use alloc::boxed::Box;
use alloc::sync::Arc;
use alloc::vec::Vec;

use crate::channel::tracker::Tracker;
use receiver::{BroadcastReceiver, ReceiverError};
use sender::BroadcastSender;
use tracker::{MultiCursorTracker, ProducerTracker, ReceiverTracker, SequentialProducerTracker};
use wait_strategy::{SpinBlockWait, WaitStrategy};

#[derive(Debug, ThisError)]
pub enum ChannelError {
    #[error("size must be a power of 2")]
    InvalidSize,
    #[error("failed to setup the channel")]
    SetupFailed(#[from] Box<dyn std::error::Error>),
    #[error("requested buffer too big")]
    BufferTooBig,
}

impl From<tracker::TrackerError> for ChannelError {
    fn from(error: tracker::TrackerError) -> Self {
        match error {
            tracker::TrackerError::InvalidSize => Self::InvalidSize,
            tracker::TrackerError::PositionTooOld => Self::SetupFailed(Box::new(error)),
        }
    }
}

impl From<ReceiverError> for ChannelError {
    fn from(error: ReceiverError) -> Self {
        match error {
            ReceiverError::RegistrationFailed(_) => Self::SetupFailed(Box::new(error)),
            _ => panic!("this path shouldn't be possible, error was {:?}", error),
        }
    }
}

pub trait Core {
    type T;
    type SendTracker: ProducerTracker;
    type ReadTracker: ReceiverTracker;
    fn sender_tracker(&self) -> &Self::SendTracker;
    fn reader_tracker(&self) -> &Self::ReadTracker;
    fn ring(&self) -> *mut Vec<Self::T>;
    fn capacity(&self) -> usize;
}

#[derive(Debug)]
pub struct Ring<T> {
    ring: *mut Vec<T>,
    capacity: usize,
    // is there a better way than events?
    sender_tracker: SequentialProducerTracker<SpinBlockWait>,
    // Reference to each reader to get their position. It should be sorted(how..?)
    reader_tracker: MultiCursorTracker<SpinBlockWait>,
}

unsafe impl<T> Send for Ring<T> {}
unsafe impl<T> Sync for Ring<T> {}

impl<T> Drop for Ring<T> {
    fn drop(&mut self) {
        let current = self.sender_tracker.current();
        unsafe {
            if current < 0 {
                (*self.ring).set_len(0);
            } else if (current as usize) < self.capacity {
                (*self.ring).set_len(current as usize);
            }
            drop(Box::from_raw(self.ring));
        }
    }
}

impl<T> Ring<T> {
    pub(crate) fn new(mut buffer_size: usize) -> Result<Self, ChannelError> {
        buffer_size = if let Some(bs) = buffer_size.checked_next_power_of_two() {
            bs
        } else {
            return Err(ChannelError::BufferTooBig);
        };
        if buffer_size > isize::MAX as usize {
            return Err(ChannelError::BufferTooBig);
        }
        let mut ring = Box::new(Vec::with_capacity(buffer_size));
        unsafe {
            ring.set_len(buffer_size);
        }

        let ring = Box::into_raw(ring);

        Ok(Self {
            ring,
            capacity: buffer_size,
            sender_tracker: SequentialProducerTracker::new(SpinBlockWait::new(0, 0)),
            reader_tracker: MultiCursorTracker::new(buffer_size, SpinBlockWait::new(0, 0))?,
        })
    }
}

impl<T> Core for Ring<T> {
    type T = T;
    type SendTracker = SequentialProducerTracker<SpinBlockWait>;
    type ReadTracker = MultiCursorTracker<SpinBlockWait>;

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

pub struct ChannelHandles<T> {
    pub sender: BroadcastSender<T>,
    pub receiver: BroadcastReceiver<T>,
}

impl<T> ChannelHandles<T> {
    fn new(sender: BroadcastSender<T>, receiver: BroadcastReceiver<T>) -> Self {
        Self { sender, receiver }
    }

    pub fn dissolve(self) -> (BroadcastSender<T>, BroadcastReceiver<T>) {
        (self.sender, self.receiver)
    }
}

///Creates a new mpmc broadcast channel returning both a sender and receiver
pub fn channel<T>(size: usize) -> Result<ChannelHandles<T>, ChannelError> {
    channel_with(size)
}

pub fn busy_channel<T>(size: usize) -> Result<ChannelHandles<T>, ChannelError> {
    channel_with(size)
}

pub fn channel_with<T>(size: usize) -> Result<ChannelHandles<T>, ChannelError> {
    let core = Arc::new(Ring::<T>::new(size)?);
    let sender = sender::BroadcastSender::from(core.clone());
    let receiver = receiver::BroadcastReceiver::try_from(core)?;
    Ok(ChannelHandles::new(sender, receiver))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channel::sender::Sender;
    use crate::Receiver;
    use pretty_assertions::assert_eq;
    use std::collections::HashMap;
    use std::sync::atomic::AtomicBool;
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
                    let mut result = HashMap::with_capacity(num_elements);
                    res.into_iter().for_each(|v| {
                        let e = result.entry(v).or_insert(0_usize);
                        *e += 1;
                    });
                    let mut missing = HashMap::new();
                    for (key, value) in &expected {
                        if let Some(val) = result.get(key) {
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
                    assert_eq!(result, expected);
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
    fn test_latency() {
        let (mut sender, mut receiver) = channel_with::<std::time::Instant>(100)
            .expect("coudln't create channel")
            .dissolve();
        let ready = Arc::new(AtomicBool::new(false));
        let ready_cloned = ready.clone();
        let th = std::thread::spawn(move || {
            ready_cloned.store(true, std::sync::atomic::Ordering::SeqCst);
            let t = receiver.recv();
            let e = t.elapsed();
            println!("duration was, {}µs", e.as_micros());
            e
        });
        while !ready.load(std::sync::atomic::Ordering::Acquire) {
            std::hint::spin_loop();
        }
        let s = std::time::Instant::now();
        while s.elapsed().as_micros() < 1000 {
            std::hint::spin_loop();
        }
        sender.send(std::time::Instant::now());
        let _ = th.join();
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
    #[ignore]
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
