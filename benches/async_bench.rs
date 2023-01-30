use async_trait::async_trait;
use criterion::async_executor::AsyncExecutor;
use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use std::fmt::{Display, Formatter};
use std::sync::mpsc::TrySendError;
use std::time::{Duration, Instant};

use nexusq::{channel, Receiver, Sender};
use workerpool::thunk::{Thunk, ThunkWorker};
use workerpool::Pool;

#[async_trait]
trait TestReceiver: Send + 'static {
    type Item;
    async fn test_recv(&mut self) -> Self::Item;
    fn another(&self) -> Self;
}

#[async_trait]
impl<T> TestReceiver for nexusq::BroadcastReceiver<T>
where
    T: 'static + Clone + Send,
{
    type Item = T;

    #[inline(always)]
    async fn test_recv(&mut self) -> Self::Item {
        loop {
            match self.async_recv().await {
                Ok(v) => return v,
                Err(_) => continue,
            }
        }
    }

    fn another(&self) -> Self {
        self.clone()
    }
}

#[async_trait]
trait TestSender<T>: Send {
    async fn test_send(&mut self, value: T);
    fn another(&self) -> Self;
}

#[async_trait]
impl<T> TestSender<T> for nexusq::BroadcastSender<T>
where
    T: 'static + Clone + Send,
{
    async fn test_send(&mut self, value: T) {
        self.async_send(value).await.expect("couldn't send");
    }

    fn another(&self) -> Self {
        self.clone()
    }
}

#[inline(always)]
async fn async_read_n(mut receiver: impl TestReceiver + 'static, num_to_read: usize) {
    for _ in 0..num_to_read {
        let _ = receiver.test_recv().await;
    }
}

#[inline(always)]
async fn async_write_n(mut sender: impl TestSender<usize> + 'static, num_to_write: usize) {
    for i in 0..num_to_write {
        sender.test_send(i).await;
    }
}

async fn nexus(num: usize, writers: usize, readers: usize, iters: u64) -> Duration {
    let mut total_duration = Duration::new(0, 0);
    for _ in 0..iters {
        let (sender, receiver) = channel(100);
        let mut receivers: Vec<_> = (0..readers - 1).map(|_| receiver.another()).collect();
        let mut senders: Vec<_> = (0..writers - 1).map(|_| sender.another()).collect();
        receivers.push(receiver);
        senders.push(sender);

        let start = Instant::now();
        let receiver_handles: Vec<_> = receivers
            .into_iter()
            .map(|r| {
                tokio::spawn(async move {
                    async_read_n(r, num * writers).await;
                })
            })
            .collect();
        let sender_handles: Vec<_> = senders
            .into_iter()
            .map(|s| {
                tokio::spawn(async move {
                    async_write_n(s, num * writers).await;
                })
            })
            .collect();

        futures::future::join_all(receiver_handles).await;
        futures::future::join_all(sender_handles).await;
        total_duration += start.elapsed();
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
    let num = 10000;
    let max_writers = 3;
    let max_readers = 3;

    let async_executor = tokio::runtime::Runtime::new().expect("couldn't start tokio");

    let mut group = c.benchmark_group("nexus");
    for readers in 1..max_readers {
        for writers in 1..max_writers {
            let input = (writers, readers);
            group.throughput(Throughput::Elements(num as u64 * writers as u64));
            group.bench_with_input(
                BenchmarkId::from_parameter(RunParam(input)),
                &input,
                |b, &input| {
                    b.to_async(&async_executor)
                        .iter_custom(|iters| black_box(nexus(num, input.0, input.1, iters)));
                },
            );
        }
    }
    group.finish();
}
criterion_group!(benches, throughput);
criterion_main!(benches);
