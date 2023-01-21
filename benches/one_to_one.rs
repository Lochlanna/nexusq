use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput};

use nexusq::channel;
use std::thread::spawn;

fn nexus(num: usize) {
    let (mut sender, mut receiver) = channel(100);
    let receiver_jh = spawn(move || {
        for _ in 0..num {
            let _ = receiver.recv().unwrap();
        }
    });
    for i in 0..num {
        sender.send(i).expect("couldn't send");
    }
    let res = receiver_jh.join();
    assert!(res.is_ok());
}

fn multiq(num: usize) {
    let (sender, receiver) = multiqueue::broadcast_queue(100);
    let receiver_jh = spawn(move || {
        for _ in 0..num {
            let v = receiver.recv();
            if v.is_err() {
                return;
            }
        }
    });
    for i in 0..num {
        while sender.try_send(i).is_err() {}
    }
    let res = receiver_jh.join();
    assert!(res.is_ok());
}

fn multiq2(num: usize) {
    let (sender, receiver) = multiqueue2::broadcast_queue(100);
    let receiver_jh = spawn(move || {
        for _ in 0..num {
            let v = receiver.recv();
            if v.is_err() {
                return;
            }
        }
    });
    for i in 0..num {
        while sender.try_send(i).is_err() {}
    }
    let res = receiver_jh.join();
    assert!(res.is_ok());
}

fn criterion_benchmark(c: &mut Criterion) {
    let num = 100000;
    let mut group = c.benchmark_group("one sender one receiver");
    group.throughput(Throughput::Elements(num as u64));
    group.bench_function(format!("nexus {}", num).as_str(), |b| {
        b.iter(|| nexus(black_box(num)))
    });
    group.bench_function(format!("multiq {}", num).as_str(), |b| {
        b.iter(|| multiq(black_box(num)))
    });
    group.bench_function(format!("multiq2 {}", num).as_str(), |b| {
        b.iter(|| multiq2(black_box(num)))
    });
    group.finish()
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);
