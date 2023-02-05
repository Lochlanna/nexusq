pub mod receiver;
pub mod sender;
pub mod tracker;

use crate::channel::tracker::Tracker;
use alloc::boxed::Box;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::AtomicIsize;
use crossbeam_utils::CachePadded;
use event_listener::Event;
use tracker::broadcast_tracker::BroadcastTracker;

#[derive(Debug)]
pub(crate) struct Core<T, TR> {
    ring: *mut Vec<T>,
    capacity: usize,
    claimed: CachePadded<AtomicIsize>,
    committed: CachePadded<AtomicIsize>,
    // is there a better way than events?
    writer_move: Event,
    // Reference to each reader to get their position. It should be sorted(how..?)
    readers: TR,
}

unsafe impl<T, TR> Send for Core<T, TR> {}
unsafe impl<T, TR> Sync for Core<T, TR> {}

impl<T, TR> Drop for Core<T, TR> {
    fn drop(&mut self) {
        unsafe {
            drop(Box::from_raw(self.ring));
        }
    }
}

impl<T, TR> Core<T, TR>
where
    TR: Tracker,
{
    pub(crate) fn new(buffer_size: usize) -> Self {
        let mut ring = Box::new(Vec::with_capacity(buffer_size));
        let capacity = ring.capacity();
        //TODO check that it's not bigger than isize::MAX
        unsafe {
            // use capacity as vec is allowed to allocate more than buffer_size if it likes so
            //we might as well use it!
            ring.set_len(capacity);
        }

        let ring = Box::into_raw(ring);

        Self {
            ring,
            capacity: capacity,
            claimed: CachePadded::new(AtomicIsize::new(0)),
            committed: CachePadded::new(AtomicIsize::new(-1)),
            writer_move: Default::default(),
            readers: TR::new(capacity),
        }
    }
}

///Creates a new mpmc broadcast channel returning both a sender and receiver
pub fn channel<T>(size: usize) -> (sender::BroadcastSender<T>, receiver::BroadcastReceiver<T>) {
    let core = Arc::new(Core::new(size));
    let inner_sender = sender::SenderCore::from(core.clone());
    let sender = sender::BroadcastSender::from(inner_sender);
    let inner_receiver = receiver::ReceiverCore::from(core);
    let receiver = receiver::BroadcastReceiver::from(inner_receiver);
    (sender, receiver)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channel::sender::Sender;
    use crate::Receiver;
    use std::prelude::v1::String;
    use std::thread::{spawn, JoinHandle};

    #[inline(always)]
    fn read_n(mut receiver: receiver::BroadcastReceiver<usize>, num_to_read: usize) -> Vec<usize> {
        let mut results = Vec::with_capacity(num_to_read);
        for _ in 0..num_to_read {
            let v = receiver.recv();
            assert!(v.is_ok());
            results.push(v.unwrap());
        }
        results
    }

    #[inline(always)]
    async fn async_read_n(
        mut receiver: receiver::BroadcastReceiver<usize>,
        num_to_read: usize,
    ) -> Vec<usize> {
        let mut results = Vec::with_capacity(num_to_read);
        for _ in 0..num_to_read {
            let v = receiver.async_recv().await;
            assert!(v.is_ok());
            results.push(v.unwrap());
        }
        results
    }

    #[inline(always)]
    fn write_n(mut sender: sender::BroadcastSender<usize>, num_to_write: usize) {
        for i in 0..num_to_write {
            sender.send(i).expect("couldn't send");
        }
    }

    #[inline(always)]
    async fn async_write_n(mut sender: sender::BroadcastSender<usize>, num_to_write: usize) {
        for i in 0..num_to_write {
            sender.async_send(i).await.expect("couldn't send");
        }
    }

    #[inline(always)]
    fn test(num_elements: usize, num_writers: usize, num_readers: usize, buffer_size: usize) {
        let (sender, receiver) = channel(buffer_size);
        let readers: Vec<JoinHandle<Vec<usize>>> = (0..num_readers)
            .map(|_| {
                let new_receiver = receiver.clone();
                spawn(move || read_n(new_receiver, num_elements * num_writers))
            })
            .collect();
        drop(receiver);
        let writers: Vec<JoinHandle<()>> = (0..num_writers)
            .map(|_| {
                let new_sender = sender.clone();
                spawn(move || {
                    write_n(new_sender, num_elements);
                })
            })
            .collect();

        for writer in writers {
            let _ = writer.join();
        }
        for reader in readers {
            let res = reader.join();
            match res {
                Ok(res) => {
                    assert_eq!(res.len(), num_elements * num_writers);
                    if num_writers == 1 {
                        let expected: Vec<usize> = (0..num_elements).collect();
                        assert_eq!(res, expected);
                    }
                }
                Err(_) => panic!("reader didnt' read enough"),
            }
        }
    }

    #[inline(always)]
    async fn test_async(
        num_elements: usize,
        num_writers: usize,
        num_readers: usize,
        buffer_size: usize,
    ) {
        let (sender, receiver) = channel(buffer_size);
        let readers: Vec<_> = (0..num_readers)
            .map(|_| {
                let new_receiver = receiver.clone();
                tokio::spawn(async_read_n(new_receiver, num_elements * num_writers))
            })
            .collect();
        drop(receiver);
        let mut senders: Vec<_> = (0..(num_writers - 1)).map(|_| sender.clone()).collect();
        senders.push(sender);
        let writers: Vec<_> = senders
            .into_iter()
            .map(|new_sender| tokio::spawn(async_write_n(new_sender, num_elements)))
            .collect();

        futures::future::join_all(writers).await;
        for reader in readers {
            let res = reader.await;
            match res {
                Ok(res) => {
                    assert_eq!(res.len(), num_elements * num_writers);
                    if num_writers == 1 {
                        let expected: Vec<usize> = (0..num_elements).collect();
                        assert_eq!(res, expected);
                    }
                }
                Err(_) => panic!("reader didnt' read enough"),
            }
        }
    }

    #[test]
    fn single_writer_single_reader() {
        let num = 5000;
        test(num, 1, 1, 10);
    }

    #[tokio::test]
    async fn async_single_writer_single_reader() {
        let num = 5000;
        test_async(num, 1, 1, 10).await;
    }

    #[test]
    fn single_writer_single_reader_clone() {
        let (mut sender, mut receiver) = channel(10);
        sender
            .send(String::from("hello world"))
            .expect("couldn't send");
        let res = receiver.recv().expect("couldn't read");
        assert_eq!(res, "hello world");
    }

    #[tokio::test]
    async fn async_single_writer_single_reader_clone() {
        let (mut sender, mut receiver) = channel(10);
        sender
            .async_send(String::from("hello world"))
            .await
            .expect("couldn't send");
        let res = receiver.async_recv().await.expect("couldn't read");
        assert_eq!(res, "hello world");
    }

    #[test]
    fn single_writer_two_reader() {
        let num = 5000;
        test(num, 1, 2, 10);
    }

    #[tokio::test]
    async fn async_single_writer_two_reader() {
        let num = 5000;
        test_async(num, 1, 2, 10).await;
    }

    #[test]
    fn two_writer_two_reader() {
        let num = 5000;
        test(num, 2, 2, 10);
    }

    #[tokio::test]
    async fn async_one_writer_five_reader() {
        let num = 5000;
        test_async(num, 1, 5, 10).await;
    }

    #[tokio::test]
    async fn async_two_writer_two_reader() {
        let num = 5000;
        test(num, 2, 2, 10);
    }

    #[tokio::test]
    async fn async_two_writer_one_reader() {
        console_subscriber::init();
        let num = 10;
        test_async(num, 1, 1, 5).await;
    }
}
