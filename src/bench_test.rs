use std::hint::black_box;
use std::time::{Duration, Instant};

use std::io::Write;
use std::sync::mpsc::TrySendError;

use crate::{channel_with, BlockWait, ChannelHandles};
use workerpool::thunk::{Thunk, ThunkWorker};
use workerpool::Pool;

trait TestReceiver<T>: Send {
    fn test_recv(&mut self) -> T;
    fn another(&self) -> Self;
}

impl<CORE> TestReceiver<CORE::T> for crate::BroadcastReceiver<CORE>
where
    CORE: crate::Core + Send + Sync,
    <CORE as crate::Core>::T: Clone,
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

impl<CORE> TestSender<CORE::T> for crate::BroadcastSender<CORE>
where
    CORE: crate::Core + Send + Sync,
    <CORE as crate::Core>::T: Send,
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
        let ChannelHandles { sender, receiver } =
            channel_with(100, BlockWait::default(), BlockWait::default())
                .expect("couldn't create channel");
        let mut receivers: Vec<_> = (0..readers - 1).map(|_| receiver.another()).collect();
        let mut senders: Vec<_> = (0..writers - 1).map(|_| sender.another()).collect();
        receivers.push(receiver);
        senders.push(sender);

        for r in receivers {
            pool.execute_to(tx.clone(), Thunk::of(move || read_n(r, num * writers)));
        }
        let senders: Vec<_> = senders
            .into_iter()
            .map(|s| Thunk::of(move || write_n(s, num)))
            .collect();
        let start = Instant::now();
        for s in senders {
            pool.execute(s);
        }
        let results = rx.iter().take(readers).count();
        total_duration += start.elapsed();
        assert_eq!(results, readers);
    }

    total_duration
}

#[test]
#[ignore]
fn test_bench() {
    let num = 50000;
    // let num = 1000;
    let writers = 2;
    let readers = 2;
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
