#![allow(dead_code)]

extern crate core;

mod broadcast;

pub use broadcast::{channel, receiver::Receiver, sender::Sender};

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread::spawn;
    use std::time;

    #[test]
    fn it_works() {
        let num = 100000;
        let (sender, mut receiver) = channel(100);
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
        let start = time::Instant::now();
        for i in 0..num {
            sender.send(i);
        }
        let res = receiver_jh.join();
        let time_taken = start.elapsed();
        assert!(res.is_ok());
        let expect: Vec<usize> = (0..num).collect();
        assert_eq!(res.unwrap(), expect);
        let throughput = num as f64 / time_taken.as_secs_f64();
        println!(
            "took {} seconds to send and receive {} values with a throughput of {}/second",
            time_taken.as_secs_f64(),
            num,
            throughput
        )
    }
}
