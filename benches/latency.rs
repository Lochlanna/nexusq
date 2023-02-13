use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use std::fmt::{Display, Formatter};
use std::sync::mpsc::TrySendError;
use std::time::{Duration, Instant};

use nexusq::{channel_with, BlockWait};
use workerpool::thunk::{Thunk, ThunkWorker};
use workerpool::Pool;

trait TestReceiver<T>: Send {
    fn test_recv(&mut self) -> T;
    fn another(&self) -> Self;
}

impl<CORE> TestReceiver<CORE::T> for nexusq::BroadcastReceiver<CORE>
where
    CORE: nexusq::Core + Send + Sync,
    <CORE as nexusq::Core>::T: Clone,
{
    #[inline(always)]
    fn test_recv(&mut self) -> CORE::T {
        self.recv()
    }

    fn another(&self) -> Self {
        self.clone()
    }
}

impl<T> TestReceiver<T> for multiqueue::BroadcastReceiver<T>
where
    T: 'static + Clone + Send,
{
    #[inline(always)]
    fn test_recv(&mut self) -> T {
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

impl<T> TestReceiver<T> for multiqueue2::BroadcastReceiver<T>
where
    T: 'static + Clone + Send + Sync,
{
    #[inline(always)]
    fn test_recv(&mut self) -> T {
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

trait TestSender<T>: Send {
    fn test_send(&mut self, value: T);
    fn another(&self) -> Self;
}

impl<CORE> TestSender<CORE::T> for nexusq::BroadcastSender<CORE>
where
    CORE: nexusq::Core + Send + Sync,
    <CORE as nexusq::Core>::T: Send,
{
    fn test_send(&mut self, value: CORE::T) {
        self.send(value);
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
fn read_n(mut receiver: impl TestReceiver<Instant> + 'static, num_to_read: usize) -> Vec<Duration> {
    let mut latencies = Vec::with_capacity(num_to_read);
    for _ in 0..num_to_read {
        let latency = receiver.test_recv().elapsed();
        latencies.push(latency);
    }
    latencies
}

#[inline(always)]
fn write_n(mut sender: impl TestSender<Instant> + 'static, num_to_write: usize) -> Vec<Duration> {
    for _ in 0..num_to_write {
        sender.test_send(Instant::now());
    }
    Default::default()
}

fn nexus(
    iterations: u64,
    writers: usize,
    readers: usize,
    pool: &Pool<ThunkWorker<Vec<Duration>>>,
    tx: &std::sync::mpsc::Sender<Vec<Duration>>,
    rx: &mut std::sync::mpsc::Receiver<Vec<Duration>>,
) -> Duration {
    let (sender, receiver) = channel_with(100, BlockWait::default(), BlockWait::default())
        .expect("couldn't create channel")
        .dissolve();

    run_test(iterations, writers, readers, pool, tx, rx, sender, receiver)
}

fn multiq2(
    iterations: u64,
    writers: usize,
    readers: usize,
    pool: &Pool<ThunkWorker<Vec<Duration>>>,
    tx: &std::sync::mpsc::Sender<Vec<Duration>>,
    rx: &mut std::sync::mpsc::Receiver<Vec<Duration>>,
) -> Duration {
    let (sender, receiver) = multiqueue2::broadcast_queue(100);

    run_test(iterations, writers, readers, pool, tx, rx, sender, receiver)
}

#[allow(clippy::too_many_arguments)]
fn run_test(
    iterations: u64,
    writers: usize,
    readers: usize,
    pool: &Pool<ThunkWorker<Vec<Duration>>>,
    tx: &std::sync::mpsc::Sender<Vec<Duration>>,
    rx: &mut std::sync::mpsc::Receiver<Vec<Duration>>,
    sender: impl TestSender<Instant> + 'static,
    receiver: impl TestReceiver<Instant> + 'static,
) -> Duration {
    let mut receivers: Vec<_> = (0..readers - 1).map(|_| receiver.another()).collect();
    let mut senders: Vec<_> = (0..writers - 1).map(|_| sender.another()).collect();
    receivers.push(receiver);
    senders.push(sender);

    let total_num_messages = iterations * (writers as u64);

    for r in receivers {
        pool.execute_to(
            tx.clone(),
            Thunk::of(move || read_n(r, total_num_messages as usize)),
        )
    }

    let senders: Vec<_> = senders
        .into_iter()
        .map(|s| Thunk::of(move || write_n(s, iterations as usize)))
        .collect();
    for s in senders {
        pool.execute(s)
    }
    rx.iter()
        .take(readers)
        .map(|r| r.into_iter().sum::<Duration>().div_f64(writers as f64))
        .sum::<Duration>()
        .div_f64(readers as f64)
}

struct RunParam((usize, usize));
impl Display for RunParam {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(format!("{},{}", self.0 .0, self.0 .1).as_str())
    }
}

fn throughput(c: &mut Criterion) {
    let max_writers = 4;
    let max_readers = 4;

    let pool = Pool::<ThunkWorker<Vec<Duration>>>::new(max_writers + max_readers);
    let (tx, mut rx) = std::sync::mpsc::channel();

    for num_writers in 1..=max_writers {
        for num_readers in 1..=max_readers {
            let mut group = c.benchmark_group("latency");
            let input = (num_writers, num_readers);
            group.bench_with_input(
                BenchmarkId::new("nexus", RunParam(input)),
                &input,
                |b, &input| {
                    b.iter_custom(|iters| {
                        black_box(nexus(iters, input.0, input.1, &pool, &tx, &mut rx))
                    });
                },
            );
            group.bench_with_input(
                BenchmarkId::new("multiq2", RunParam(input)),
                &input,
                |b, &input| {
                    b.iter_custom(|iters| {
                        black_box(multiq2(iters, input.0, input.1, &pool, &tx, &mut rx))
                    });
                },
            );
            group.finish();
        }
    }
}
criterion_group!(benches, throughput);
criterion_main!(benches);
