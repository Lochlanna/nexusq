#![allow(dead_code)]

mod broadcast;

pub use broadcast::{channel, receiver::Receiver, sender::Sender};

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread::{spawn, JoinHandle};

    #[inline(always)]
    fn read_n(mut receiver: Receiver<usize>, num_to_read: usize) -> Vec<usize> {
        let mut results = Vec::with_capacity(num_to_read);
        for _ in 0..num_to_read {
            let v = receiver.recv();
            assert!(v.is_ok());
            results.push(v.unwrap());
        }
        results
    }

    #[inline(always)]
    fn write_n(mut sender: Sender<usize>, num_to_write: usize) {
        for i in 0..num_to_write {
            sender.send(i);
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
}
