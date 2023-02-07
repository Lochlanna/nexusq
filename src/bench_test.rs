use std::time::{Duration, Instant};

use crate::{Receiver, Sender};
use std::io::Write;
use std::println;
use std::vec::Vec;
use workerpool::thunk::{Thunk, ThunkWorker};
use workerpool::Pool;

trait TestReceiver: Send + 'static {
    type Item;
    fn test_recv(&mut self) -> Self::Item;
    fn another(&self) -> Self;
}

impl<X> TestReceiver for X
where
    X: Receiver<usize> + Send + 'static,
{
    type Item = usize;

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

trait TestSender<T>: Send + 'static {
    fn test_send(&mut self, value: T);
    fn another(&self) -> Self;
}

impl<X> TestSender<usize> for X
where
    X: Sender<usize> + Send + 'static,
{
    fn test_send(&mut self, value: usize) {
        self.send(value);
    }

    fn another(&self) -> Self {
        self.clone()
    }
}

#[inline(always)]
fn read_n(mut receiver: impl TestReceiver + 'static, num_to_read: usize) {
    for _ in 0..num_to_read {
        let _ = receiver.test_recv();
    }
}

#[inline(always)]
fn write_n(mut sender: impl TestSender<usize> + 'static, num_to_write: usize) {
    for i in 0..num_to_write {
        sender.test_send(i);
    }
}

fn nexus(
    num: usize,
    writers: usize,
    readers: usize,
    pool: &Pool<ThunkWorker<()>>,
    tx: &std::sync::mpsc::Sender<()>,
    rx: &mut std::sync::mpsc::Receiver<()>,
    iters: u64,
) -> Duration {
    let mut total_duration = Duration::new(0, 0);
    for _ in 0..iters {
        let (sender, receiver) = crate::channel(100);
        let mut receivers: Vec<_> = (0..readers - 1).map(|_| receiver.another()).collect();
        let mut senders: Vec<_> = (0..writers - 1).map(|_| sender.another()).collect();
        receivers.push(receiver);
        senders.push(sender);

        let start = Instant::now();
        for r in receivers {
            pool.execute_to(tx.clone(), Thunk::of(move || read_n(r, num * writers)))
        }
        for s in senders {
            pool.execute_to(tx.clone(), Thunk::of(move || write_n(s, num)))
        }
        let num = rx.iter().take(readers + writers).count();
        total_duration += start.elapsed();
        assert_eq!(num, readers + writers);
    }

    total_duration
}

#[test]
fn test_bench() {
    let num = 50000;
    // let num = 1000;
    let writers = 3;
    let readers = 3;
    let iterations = 100;

    let pool = Pool::<ThunkWorker<()>>::new(writers + readers);
    let (tx, mut rx) = std::sync::mpsc::channel();
    for _ in 1..=1 {
        let duration = nexus(num, writers, readers, &pool, &tx, &mut rx, iterations);
        let throughput =
            (num * writers * iterations as usize) as f64 / duration.as_secs_f64() / 1000000_f64;
        println!("{readers} throughput is {throughput} million/second");
        let _ = std::io::stdout().flush();
    }
}
