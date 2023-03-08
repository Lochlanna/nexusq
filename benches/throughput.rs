mod shared;
use shared::*;

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use std::fmt::{Display, Formatter};
use std::time::{Duration, Instant};

use nexusq::channel_with;
use workerpool::thunk::{Thunk, ThunkWorker};
use workerpool::Pool;

#[inline(always)]
fn read_n(mut receiver: impl TestReceiver<usize> + 'static, num_to_read: usize) {
    for _ in 0..num_to_read {
        black_box(receiver.test_recv());
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
        let (sender, receiver) = channel_with(100)
            .expect("couldn't create channel")
            .dissolve();

        total_duration += run_test(num, writers, readers, pool, tx, rx, sender, receiver);
    }

    total_duration
}

fn multiq2(
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
        let (sender, receiver) = multiqueue2::broadcast_queue(100);

        total_duration += run_test(num, writers, readers, pool, tx, rx, sender, receiver);
    }

    total_duration
}

#[allow(clippy::too_many_arguments)]
fn run_test(
    num: usize,
    writers: usize,
    readers: usize,
    pool: &Pool<ThunkWorker<()>>,
    tx: &std::sync::mpsc::Sender<()>,
    rx: &mut std::sync::mpsc::Receiver<()>,
    sender: impl TestSender<usize> + 'static,
    receiver: impl TestReceiver<usize> + 'static,
) -> Duration {
    let mut receivers: Vec<_> = (0..readers - 1).map(|_| receiver.another()).collect();
    let mut senders: Vec<_> = (0..writers - 1).map(|_| sender.another()).collect();
    receivers.push(receiver);
    senders.push(sender);

    for r in receivers {
        pool.execute_to(tx.clone(), Thunk::of(move || read_n(r, num * writers)))
    }

    let senders: Vec<_> = senders
        .into_iter()
        .map(|s| Thunk::of(move || write_n(s, num)))
        .collect();
    let start = Instant::now();
    for s in senders {
        pool.execute(s)
    }
    let results = rx.iter().take(readers).count();
    let duration = start.elapsed();
    assert_eq!(results, readers);
    duration
}

struct RunParam((usize, usize));
impl Display for RunParam {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(format!("{},{}", self.0 .0, self.0 .1).as_str())
    }
}

fn throughput(c: &mut Criterion) {
    let num_elements = 20000;
    let max_writers = 4;
    let max_readers = 4;

    let pool = Pool::<ThunkWorker<()>>::new(max_writers + max_readers);
    let (tx, mut rx) = std::sync::mpsc::channel();

    for num_writers in 1..=max_writers {
        for num_readers in 1..=max_readers {
            let mut group = c.benchmark_group("throughput");
            let input = (num_writers, num_readers);
            group.throughput(Throughput::Elements(
                num_elements as u64 * num_writers as u64,
            ));
            group.bench_with_input(
                BenchmarkId::new("nexus", RunParam(input)),
                &input,
                |b, &input| {
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
                },
            );
            group.bench_with_input(
                BenchmarkId::new("multiq2", RunParam(input)),
                &input,
                |b, &input| {
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
                },
            );
            group.finish();
        }
    }
}
criterion_group!(benches, throughput);
criterion_main!(benches);
