use std::fmt::{Display, Formatter};
use std::sync::mpsc::TrySendError;
use std::thread;
use std::time::{Duration, Instant};

use crate::{Receiver, Sender};
use workerpool::thunk::{Thunk, ThunkWorker};
use workerpool::Pool;

trait TestReceiver: Send + 'static {
    type Item;
    fn test_recv(&mut self) -> Self::Item;
    fn another(&self) -> Self;
}

impl<T> TestReceiver for crate::BroadcastReceiver<T>
where
    T: 'static + Clone + Send,
{
    type Item = T;

    #[inline(always)]
    fn test_recv(&mut self) -> Self::Item {
        loop {
            match self.recv() {
                Ok(v) => return v,
                Err(_) => continue,
            }
        }
    }

    fn another(&self) -> Self {
        self.clone()
    }
}

impl<T> TestReceiver for multiqueue::BroadcastReceiver<T>
where
    T: 'static + Clone + Send,
{
    type Item = T;
    #[inline(always)]
    fn test_recv(&mut self) -> Self::Item {
        loop {
            let res = self.recv();
            match res {
                Ok(v) => return v,
                Err(_) => continue,
            }
        }
    }

    fn another(&self) -> Self {
        self.add_stream()
    }
}

impl<T> TestReceiver for multiqueue2::BroadcastReceiver<T>
where
    T: 'static + Clone + Send + Sync,
{
    type Item = T;
    #[inline(always)]
    fn test_recv(&mut self) -> Self::Item {
        loop {
            let res = self.recv();
            match res {
                Ok(v) => return v,
                Err(_) => continue,
            }
        }
    }

    fn another(&self) -> Self {
        self.add_stream()
    }
}

trait TestSender<T>: Send + 'static {
    fn test_send(&mut self, value: T);
    fn another(&self) -> Self;
}

impl<T> TestSender<T> for crate::BroadcastSender<T>
where
    T: 'static + Clone + Send,
{
    fn test_send(&mut self, value: T) {
        self.send(value).expect("couldn't send");
    }

    fn another(&self) -> Self {
        self.clone()
    }
}

impl<T> TestSender<T> for multiqueue::BroadcastSender<T>
where
    T: 'static + Clone + Send,
{
    #[inline(always)]
    fn test_send(&mut self, mut value: T) {
        while let Err(err) = self.try_send(value) {
            match err {
                TrySendError::Full(v) => value = v,
                TrySendError::Disconnected(_) => panic!("multiq disconnected"),
            }
        }
    }

    fn another(&self) -> Self {
        self.clone()
    }
}

impl<T> TestSender<T> for multiqueue2::BroadcastSender<T>
where
    T: 'static + Clone + Send + Sync,
{
    #[inline(always)]
    fn test_send(&mut self, mut value: T) {
        while let Err(err) = self.try_send(value) {
            match err {
                TrySendError::Full(v) => value = v,
                TrySendError::Disconnected(_) => panic!("multiq disconnected"),
            }
        }
    }

    fn another(&self) -> Self {
        self.clone()
    }
}

#[inline(always)]
fn read_n(mut receiver: impl TestReceiver + 'static, num_to_read: usize) -> bool {
    for i in 0..num_to_read {
        let _ = receiver.test_recv();
    }
    true
}

#[inline(always)]
fn write_n(mut sender: impl TestSender<usize> + 'static, num_to_write: usize) -> bool {
    for i in 0..num_to_write {
        sender.test_send(i);
    }
    true
}

#[allow(clippy::too_many_arguments)]
#[inline(always)]
fn test(
    num_elements: usize,
    num_writers: usize,
    num_readers: usize,
    sender: impl TestSender<usize>,
    receiver: impl TestReceiver,
    pool: &Pool<ThunkWorker<bool>>,
    tx: &std::sync::mpsc::Sender<bool>,
    rx: &mut std::sync::mpsc::Receiver<bool>,
) {
    for _ in 0..num_readers {
        let new_receiver = receiver.another();
        pool.execute_to(
            tx.clone(),
            Thunk::of(move || read_n(new_receiver, num_elements * num_writers)),
        )
    }
    drop(receiver);
    thread::sleep(Duration::from_millis(10));

    for _ in 0..num_writers {
        let new_sender = sender.another();
        pool.execute_to(
            tx.clone(),
            Thunk::of(move || write_n(new_sender, num_elements)),
        )
    }
    drop(sender);
    let num = rx.iter().take(num_readers + num_writers).count();
    assert_eq!(num, num_readers + num_writers);
}

fn nexus(
    num: usize,
    writers: usize,
    readers: usize,
    pool: &Pool<ThunkWorker<bool>>,
    tx: &std::sync::mpsc::Sender<bool>,
    rx: &mut std::sync::mpsc::Receiver<bool>,
) {
    let (sender, receiver) = crate::channel(100);
    test(num, writers, readers, sender, receiver, pool, tx, rx);
}

fn multiq(
    num: usize,
    writers: usize,
    readers: usize,
    pool: &Pool<ThunkWorker<bool>>,
    tx: &std::sync::mpsc::Sender<bool>,
    rx: &mut std::sync::mpsc::Receiver<bool>,
) {
    let (sender, receiver) = multiqueue::broadcast_queue(100);
    test(num, writers, readers, sender, receiver, pool, tx, rx);
}

fn multiq2(
    num: usize,
    writers: usize,
    readers: usize,
    pool: &Pool<ThunkWorker<bool>>,
    tx: &std::sync::mpsc::Sender<bool>,
    rx: &mut std::sync::mpsc::Receiver<bool>,
) {
    let (sender, receiver) = multiqueue2::broadcast_queue(100);
    test(num, writers, readers, sender, receiver, pool, tx, rx);
}

struct RunParam((usize, usize));
impl Display for RunParam {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(format!("{},{}", self.0 .0, self.0 .1).as_str())
    }
}

#[test]
fn test_bench() {
    let num = 1000;
    let writers = 2;
    let readers = 1;
    let iterations = 10;

    let pool = Pool::<ThunkWorker<bool>>::new(writers + readers);
    let (mut tx, mut rx) = std::sync::mpsc::channel();

    let start = Instant::now();
    for _ in 0..iterations {
        nexus(num, writers, readers, &pool, &tx, &mut rx)
    }
    let elapsed = start.elapsed();
    let giga_throughput = (num * writers * iterations) as f64 / elapsed.as_secs_f64() / 1000000_f64;
    println!("throughput is {} million/second", giga_throughput);
}
