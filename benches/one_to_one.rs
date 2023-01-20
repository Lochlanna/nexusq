use criterion::{black_box, criterion_group, criterion_main, Criterion};

use nexusq::channel;
use std::thread::spawn;

fn nexus(num: usize) {
    let (mut sender, mut receiver) = channel(100);
    let receiver_jh = spawn(move || {
        let mut values = Vec::with_capacity(num);
        for _ in 0..num {
            loop {
                let v = receiver.try_read_next();
                if v.is_err() {
                    continue;
                }
                values.push(v.unwrap());
                break;
            }
        }
        values
    });
    for i in 0..num {
        sender.send(i);
    }
    let res = receiver_jh.join();
    assert!(res.is_ok());
    let expect: Vec<usize> = (0..num).collect();
    assert_eq!(res.unwrap(), expect);
}

fn multiq(num: usize) {
    let (sender, receiver) = multiqueue::broadcast_queue(100);
    let receiver_jh = spawn(move || {
        let mut values = Vec::with_capacity(num);
        for _ in 0..num {
            let v = receiver.recv();
            if v.is_err() {
                return values;
            }
            values.push(v.unwrap());
        }
        values
    });
    for i in 0..num {
        while sender.try_send(i).is_err() {}
    }
    let res = receiver_jh.join();
    assert!(res.is_ok());
    let expect: Vec<usize> = (0..num).collect();
    assert_eq!(res.unwrap(), expect);
}

fn multiq2(num: usize) {
    let (sender, receiver) = multiqueue2::broadcast_queue(100);
    let receiver_jh = spawn(move || {
        let mut values = Vec::with_capacity(num);
        for _ in 0..num {
            let v = receiver.recv();
            if v.is_err() {
                return values;
            }
            values.push(v.unwrap());
        }
        values
    });
    for i in 0..num {
        while sender.try_send(i).is_err() {}
    }
    let res = receiver_jh.join();
    assert!(res.is_ok());
    let expect: Vec<usize> = (0..num).collect();
    assert_eq!(res.unwrap(), expect);
}

fn criterion_benchmark(c: &mut Criterion) {
    let num = 100000;
    c.bench_function(format!("nexus {}", num).as_str(), |b| {
        b.iter(|| nexus(black_box(num)))
    });
    c.bench_function(format!("multiq {}", num).as_str(), |b| {
        b.iter(|| multiq(black_box(num)))
    });
    c.bench_function(format!("multiq2 {}", num).as_str(), |b| {
        b.iter(|| multiq2(black_box(num)))
    });
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);
