#![allow(dead_code)]

extern crate core;

mod broadcast;

pub use broadcast::{channel, receiver::Receiver, sender::Sender};

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread::spawn;
    #[test]
    fn it_works() {
        let num = 100;
        let (sender, mut receiver) = channel(10);
        let sender_jh = spawn(move || {
            for i in 0..num {
                sender.send(i);
            }
        });
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
        let _ = sender_jh.join();
        let res = receiver_jh.join();
        assert!(res.is_ok());
        let expect: Vec<usize> = (0..num).collect();
        assert_eq!(res.unwrap(), expect)
    }
}
