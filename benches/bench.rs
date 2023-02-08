use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput};
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
fn read_n(mut receiver: impl TestReceiver<usize> + 'static, num_to_read: usize) -> Vec<usize> {
    let mut res = Vec::with_capacity(num_to_read);
    for _ in 0..num_to_read {
        res.push(receiver.test_recv());
    }
    res
}

#[inline(always)]
fn write_n(mut sender: impl TestSender<usize> + 'static, num_to_write: usize) -> Vec<usize> {
    for i in 0..num_to_write {
        sender.test_send(i);
    }
    Default::default()
}

fn nexus(
    num: usize,
    writers: usize,
    readers: usize,
    pool: &Pool<ThunkWorker<Vec<usize>>>,
    tx: &std::sync::mpsc::Sender<Vec<usize>>,
    rx: &mut std::sync::mpsc::Receiver<Vec<usize>>,
    iters: u64,
) -> Duration {
    let mut total_duration = Duration::new(0, 0);
    for _ in 0..iters {
        // let (sender, receiver) = busy_channel(100);
        let (sender, receiver) = channel_with(100, BlockWait::new(0, 0), BlockWait::new(0, 0));
        let mut receivers: Vec<_> = (0..readers - 1).map(|_| receiver.another()).collect();
        let mut senders: Vec<_> = (0..writers - 1).map(|_| sender.another()).collect();
        receivers.push(receiver);
        senders.push(sender);

        let start = Instant::now();
        for r in receivers {
            pool.execute_to(tx.clone(), Thunk::of(move || read_n(r, num * writers)))
        }
        for s in senders {
            pool.execute(Thunk::of(move || write_n(s, num)))
        }
        let results = rx.iter().take(readers);
        total_duration += start.elapsed();
        for result in results {
            assert_eq!(result.len(), num * writers);
            black_box(result);
        }
    }

    total_duration
}

fn multiq(
    num: usize,
    writers: usize,
    readers: usize,
    pool: &Pool<ThunkWorker<Vec<usize>>>,
    tx: &std::sync::mpsc::Sender<Vec<usize>>,
    rx: &mut std::sync::mpsc::Receiver<Vec<usize>>,
    iters: u64,
) -> Duration {
    let mut total_duration = Duration::new(0, 0);
    for _ in 0..iters {
        let (sender, receiver) = multiqueue::broadcast_queue(100);

        let mut receivers: Vec<_> = (0..readers - 1).map(|_| receiver.another()).collect();
        let mut senders: Vec<_> = (0..writers - 1).map(|_| sender.another()).collect();
        receivers.push(receiver);
        senders.push(sender);

        let start = Instant::now();
        for r in receivers {
            pool.execute_to(tx.clone(), Thunk::of(move || read_n(r, num * writers)))
        }

        for s in senders {
            pool.execute(Thunk::of(move || write_n(s, num)))
        }
        let results = rx.iter().take(readers);
        total_duration += start.elapsed();
        for result in results {
            assert_eq!(result.len(), num * writers);
            black_box(result);
        }
    }

    total_duration
}

fn multiq2(
    num: usize,
    writers: usize,
    readers: usize,
    pool: &Pool<ThunkWorker<Vec<usize>>>,
    tx: &std::sync::mpsc::Sender<Vec<usize>>,
    rx: &mut std::sync::mpsc::Receiver<Vec<usize>>,
    iters: u64,
) -> Duration {
    let mut total_duration = Duration::new(0, 0);
    for _ in 0..iters {
        let (sender, receiver) = multiqueue2::broadcast_queue(100);

        let mut receivers: Vec<_> = (0..readers - 1).map(|_| receiver.another()).collect();
        let mut senders: Vec<_> = (0..writers - 1).map(|_| sender.another()).collect();
        receivers.push(receiver);
        senders.push(sender);

        let start = Instant::now();
        for r in receivers {
            pool.execute_to(tx.clone(), Thunk::of(move || read_n(r, num * writers)))
        }

        for s in senders {
            pool.execute(Thunk::of(move || write_n(s, num)))
        }
        let results = rx.iter().take(readers);
        total_duration += start.elapsed();
        for result in results {
            assert_eq!(result.len(), num * writers);
            black_box(result);
        }
    }

    total_duration
}

struct RunParam((usize, usize));
impl Display for RunParam {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(format!("{},{}", self.0 .0, self.0 .1).as_str())
    }
}

fn throughput(c: &mut Criterion) {
    let num_elements = 10000;
    let max_writers = 2;
    let max_readers = 2;

    let pool = Pool::<ThunkWorker<Vec<usize>>>::new(max_writers + max_readers);
    let (tx, mut rx) = std::sync::mpsc::channel();

    for num_writers in 2..=max_writers {
        for num_readers in 2..=max_readers {
            let mut group = c.benchmark_group(format!("{num_writers}w, {num_readers}r"));
            let input = (num_writers, num_readers);
            group.throughput(Throughput::Elements(
                num_elements as u64 * num_writers as u64,
            ));
            group.bench_with_input("nexus", &input, |b, &input| {
                b.iter_custom(|iters| {
                    black_box(nexus(
                        num_elements,
                        input.0,
                        input.1,
                        &pool,
                        &tx,
                        &mut rx,
                        iters,
                    ))
                });
            });
            // group.bench_with_input("multiq", &input, |b, &input| {
            //     b.iter_custom(|iters| {
            //         black_box(multiq(
            //             num_elements,
            //             input.0,
            //             input.1,
            //             &pool,
            //             &tx,
            //             &mut rx,
            //             iters,
            //         ))
            //     });
            // });
            group.bench_with_input("multiq2", &input, |b, &input| {
                b.iter_custom(|iters| {
                    black_box(multiq2(
                        num_elements,
                        input.0,
                        input.1,
                        &pool,
                        &tx,
                        &mut rx,
                        iters,
                    ))
                });
            });
            group.finish();
        }
    }
}
criterion_group!(benches, throughput);
criterion_main!(benches);
