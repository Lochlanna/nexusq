pub mod receiver;
mod receiver_tracker;
pub mod sender;

use crossbeam_utils::CachePadded;
use event_listener::Event;
use receiver_tracker::ReceiverTracker;
use std::sync::atomic::{AtomicIsize, AtomicUsize};
use std::sync::Arc;

#[derive(Debug)]
pub(crate) struct Core<T> {
    ring: *mut Vec<T>,
    capacity: isize,
    claimed: CachePadded<AtomicIsize>,
    committed: CachePadded<AtomicIsize>,
    // is there a better way than events?
    reader_move: Event,
    writer_move: Event,
    // Reference to each reader to get their position. It should be sorted(how..?)
    readers: ReceiverTracker,
}

unsafe impl<T> Send for Core<T> {}
unsafe impl<T> Sync for Core<T> {}

impl<T> Drop for Core<T> {
    fn drop(&mut self) {
        unsafe {
            drop(Box::from_raw(self.ring));
        }
    }
}

impl<T> Core<T> {
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
            capacity: capacity as isize,
            claimed: CachePadded::new(AtomicIsize::new(0)),
            committed: CachePadded::new(AtomicIsize::new(-1)),
            reader_move: Default::default(),
            writer_move: Default::default(),
            readers: Default::default(),
        }
    }
}

pub fn channel<T>(size: usize) -> (sender::Sender<T>, receiver::Receiver<T>) {
    let core = Arc::new(Core::new(size));
    let sender = sender::Sender::from(core.clone());
    let receiver = receiver::Receiver::from(core);
    (sender, receiver)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread::{spawn, JoinHandle};

    #[inline(always)]
    fn read_n(mut receiver: receiver::Receiver<usize>, num_to_read: usize) -> Vec<usize> {
        let mut results = Vec::with_capacity(num_to_read);
        for _ in 0..num_to_read {
            let v = receiver.recv();
            assert!(v.is_ok());
            results.push(v.unwrap());
        }
        results
    }

    #[inline(always)]
    fn write_n(mut sender: sender::Sender<usize>, num_to_write: usize) {
        for i in 0..num_to_write {
            sender.send(i).expect("couldn't send");
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

    #[test]
    fn single_writer_single_reader() {
        let num = 5000;
        test(num, 1, 1, 10);
    }

    #[test]
    fn single_writer_two_reader() {
        let num = 5000;
        test(num, 1, 2, 10);
    }

    #[test]
    fn two_writer_two_reader() {
        let num = 5000;
        test(num, 2, 2, 10);
    }

    #[test]
    fn test_batch_read() {
        let (mut sender, mut receiver) = channel(10);
        let expected: Vec<i32> = (0..8).collect();
        for i in &expected {
            sender.send(*i).expect("couldn't send");
        }
        let mut result = Vec::new();
        receiver
            .batch_recv(&mut result)
            .expect("receiver was okay!");

        assert_eq!(result, expected)
    }

    #[test]
    fn test_read_entire_buffer() {
        let (mut sender, mut receiver) = channel(10);
        let expected: Vec<i32> = (0..10).collect();
        for i in &expected {
            sender.send(*i).expect("couldn't send");
        }
        let mut result = Vec::new();
        receiver
            .batch_recv(&mut result)
            .expect("receiver was okay!");

        assert_eq!(result, expected)
    }

    #[test]
    fn test_wrapping_batch() {
        let (mut sender, mut receiver) = channel(10);
        for i in 0..10 {
            sender.send(i).expect("couldn't send");
        }
        let _ = receiver.recv();
        let _ = receiver.recv();
        sender.send(10).expect("couldn't send");
        sender.send(11).expect("couldn't send");
        let expected: Vec<i32> = (2..12).collect();
        let mut result = Vec::new();
        receiver
            .batch_recv(&mut result)
            .expect("receiver was okay!");

        assert_eq!(result, expected)
    }

    #[test]
    fn batch_write() {
        let (mut sender, mut receiver) = channel(50);
        let expected: Vec<i32> = (0..12).collect();
        sender.send_batch(expected).expect("send failed");
        let mut result = Vec::new();
        receiver
            .batch_recv(&mut result)
            .expect("receiver was okay!");

        let expected: Vec<i32> = (0..12).collect();
        assert_eq!(result, expected)
    }

    #[test]
    fn batch_write_wrapping() {
        let (mut sender, mut receiver) = channel(10);
        sender.send(0).expect("coulnd't send");
        sender.send(1).expect("coulnd't send");
        let _ = receiver.recv().expect("couldn't receive");
        let _ = receiver.recv().expect("couldn't receive");
        let expected: Vec<i32> = (2..12).collect();
        sender.send_batch(expected).expect("send failed");
        let mut result = Vec::new();
        receiver
            .batch_recv(&mut result)
            .expect("receiver was okay!");

        let expected: Vec<i32> = (2..12).collect();
        assert_eq!(result, expected)
    }
}
