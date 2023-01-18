#![allow(dead_code)]

mod disruptor;

use disruptor::DisruptorCore;
pub use disruptor::{receiver::Receiver, sender::Sender};
use std::sync::Arc;

pub fn channel<T>(size: usize) -> (Sender<T>, Receiver<T>) {
    let core = Arc::new(DisruptorCore::new(size));
    let sender = Sender::from(core.clone());
    let receiver = Receiver::from(core);
    (sender, receiver)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_works() {
        let (sender, mut receiver) = channel(10);
        sender.send(42);
        let res = receiver.try_read_next();
        assert!(res.is_ok());
        let res = res.unwrap();
        assert_eq!(res, 42);
    }
}
