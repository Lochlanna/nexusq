use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use std::fmt::{Display, Formatter};
use std::sync::mpsc::TrySendError;

use nexusq::{channel, Receiver, Sender};
use workerpool::thunk::{Thunk, ThunkWorker};
use workerpool::Pool;

trait TestReceiver: Send + 'static {
    type Item;
    fn test_recv(&mut self) -> Self::Item;
    fn another(&self) -> Self;
}

impl<T> TestReceiver for nexusq::BroadcastReceiver<T>
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

impl<T> TestSender<T> for nexusq::BroadcastSender<T>
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

#[allow(clippy::too_many_arguments)]
#[inline(always)]
fn test(
    num_elements: usize,
    num_writers: usize,
    num_readers: usize,
    sender: impl TestSender<usize>,
    receiver: impl TestReceiver,
    pool: &Pool<ThunkWorker<()>>,
    tx: &std::sync::mpsc::Sender<()>,
    rx: &mut std::sync::mpsc::Receiver<()>,
) {
    for _ in 0..num_readers {
        let new_receiver = receiver.another();
        pool.execute_to(
            tx.clone(),
            Thunk::of(move || read_n(new_receiver, num_elements * num_writers)),
        )
    }
    drop(receiver);

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
    pool: &Pool<ThunkWorker<()>>,
    tx: &std::sync::mpsc::Sender<()>,
    rx: &mut std::sync::mpsc::Receiver<()>,
) {
    let (sender, receiver) = channel(100);
    test(num, writers, readers, sender, receiver, pool, tx, rx);
}

fn multiq(
    num: usize,
    writers: usize,
    readers: usize,
    pool: &Pool<ThunkWorker<()>>,
    tx: &std::sync::mpsc::Sender<()>,
    rx: &mut std::sync::mpsc::Receiver<()>,
) {
    let (sender, receiver) = multiqueue::broadcast_queue(100);
    test(num, writers, readers, sender, receiver, pool, tx, rx);
}

fn multiq2(
    num: usize,
    writers: usize,
    readers: usize,
    pool: &Pool<ThunkWorker<()>>,
    tx: &std::sync::mpsc::Sender<()>,
    rx: &mut std::sync::mpsc::Receiver<()>,
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

fn throughput(c: &mut Criterion) {
    let num = 10000;
    let max_writers = 3;
    let max_readers = 3;

    let pool = Pool::<ThunkWorker<()>>::new(max_writers + max_readers);
    let (mut tx, mut rx) = std::sync::mpsc::channel();

    let mut group = c.benchmark_group("nexus");
    for readers in 1..max_readers {
        for writers in 1..max_writers {
            let input = (writers, readers);
            println!("input is {}", RunParam(input));
            group.throughput(Throughput::Elements(num as u64 * writers as u64));
            group.bench_with_input(
                BenchmarkId::from_parameter(RunParam(input)),
                &input,
                |b, &input| {
                    b.iter(|| black_box(nexus(num, input.0, input.1, &pool, &tx, &mut rx)));
                },
            );
        }
    }
    group.finish();

    let mut group = c.benchmark_group("multiq");
    for readers in 1..max_readers {
        for writers in 1..max_writers {
            let input = (writers, readers);
            group.throughput(Throughput::Elements(num as u64 * writers as u64));
            group.bench_with_input(
                BenchmarkId::from_parameter(RunParam(input)),
                &input,
                |b, &input| {
                    b.iter(|| black_box(multiq(num, input.0, input.1, &pool, &mut tx, &mut rx)));
                },
            );
        }
    }
    group.finish();

    let mut group = c.benchmark_group("multiq2");
    for readers in 1..max_readers {
        for writers in 1..max_writers {
            let input = (writers, readers);
            group.throughput(Throughput::Elements(num as u64 * writers as u64));
            group.bench_with_input(
                BenchmarkId::from_parameter(RunParam(input)),
                &input,
                |b, &input| {
                    b.iter(|| black_box(multiq2(num, input.0, input.1, &pool, &mut tx, &mut rx)));
                },
            );
        }
    }
    group.finish();
}
criterion_group!(benches, throughput);
criterion_main!(benches);
