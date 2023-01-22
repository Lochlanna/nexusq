use super::*;
use crate::channel::tracker::Tracker;
use std::mem::forget;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SenderError {
    #[error("placeholder error")]
    Placeholder,
}

#[derive(Debug, Clone)]
pub struct Sender<T> {
    disruptor: Arc<Core<T>>,
    capacity: isize,
    cached_slowest_reader: isize,
}

impl<T> From<Arc<Core<T>>> for Sender<T> {
    fn from(disruptor: Arc<Core<T>>) -> Self {
        let capacity = disruptor.capacity;
        Self {
            disruptor,
            capacity,
            cached_slowest_reader: -1,
        }
    }
}

impl<T> Sender<T>
where
    T: Send,
{
    //TODO this can probably be cut down / optimised a bit...
    fn claim(&mut self, num_to_claim: isize) -> Result<isize, SenderError> {
        if num_to_claim > self.capacity {
            return Err(SenderError::Placeholder);
        }
        let claimed = self
            .disruptor
            .claimed
            .fetch_add(num_to_claim, Ordering::Release);
        //TODO we create this every time even though we hopefully wont use it every time

        let tail = claimed - self.capacity;

        if self.cached_slowest_reader != -1 && self.cached_slowest_reader > tail {
            return Ok(claimed);
        }

        let mut slowest_reader = self.disruptor.readers.slowest(tail);

        while slowest_reader <= tail {
            let listener = self.disruptor.reader_move.listen();
            // the reader may have moved before we managed to register the listener so
            // we need to check again before we wait. If a reader moves between now and when
            // we call wait it should return immediately
            slowest_reader = self.disruptor.readers.slowest(tail);
            if slowest_reader > tail {
                return Ok(claimed);
            }
            listener.wait();
            slowest_reader = self.disruptor.readers.slowest(tail);
        }
        // TODO check if there is another writer writing to a different ID but the same cell.
        if slowest_reader > tail {
            self.cached_slowest_reader = slowest_reader;
        }

        Ok(claimed)
    }

    async fn async_claim(&mut self, num_to_claim: isize) -> Result<isize, SenderError> {
        if num_to_claim > self.capacity {
            return Err(SenderError::Placeholder);
        }
        let claimed = self
            .disruptor
            .claimed
            .fetch_add(num_to_claim, Ordering::Release);
        //TODO we create this every time even though we hopefully wont use it every time

        let tail = claimed - self.capacity;

        if self.cached_slowest_reader != -1 && self.cached_slowest_reader > tail {
            return Ok(claimed);
        }

        let mut slowest_reader = self.disruptor.readers.slowest(tail);

        while slowest_reader <= tail {
            let listener = self.disruptor.reader_move.listen();
            // the reader may have moved before we managed to register the listener so
            // we need to check again before we wait. If a reader moves between now and when
            // we call wait it should return immediately
            slowest_reader = self.disruptor.readers.slowest(tail);
            if slowest_reader > tail {
                return Ok(claimed);
            }
            listener.await;
            slowest_reader = self.disruptor.readers.slowest(tail);
        }
        // TODO check if there is another writer writing to a different ID but the same cell.
        if slowest_reader > tail {
            self.cached_slowest_reader = slowest_reader;
        }

        Ok(claimed)
    }
    pub fn send(&mut self, value: T) -> Result<(), SenderError> {
        let claimed_id = self.claim(1)?;
        let index = claimed_id as usize % self.capacity as usize;

        let old_value;
        unsafe {
            old_value = std::mem::replace((*self.disruptor.ring).get_unchecked_mut(index), value)
        }

        // TODO what's the optimisation here
        // wait for writers to catch up and commit their transactions
        while self
            .disruptor
            .committed
            .compare_exchange_weak(
                claimed_id - 1,
                claimed_id,
                Ordering::Release,
                Ordering::Relaxed,
            )
            .is_err()
        {}
        // Notify other threads that a value has been written
        self.disruptor.writer_move.notify(usize::MAX);

        // We do this at the end to ensure that we're not worrying about wierd drop functions or
        // allocations happening during the critical path
        if (claimed_id as usize) < self.capacity as usize {
            forget(old_value);
        } else {
            drop(old_value);
        }
        Ok(())
    }
    pub async fn async_send(&mut self, value: T) -> Result<(), SenderError> {
        let claimed_id = self.async_claim(1).await?;
        let index = claimed_id as usize % self.capacity as usize;

        let old_value;
        unsafe {
            old_value = std::mem::replace((*self.disruptor.ring).get_unchecked_mut(index), value)
        }

        // TODO what's the optimisation here
        // wait for writers to catch up and commit their transactions
        while self
            .disruptor
            .committed
            .compare_exchange_weak(
                claimed_id - 1,
                claimed_id,
                Ordering::Release,
                Ordering::Relaxed,
            )
            .is_err()
        {}
        // Notify other threads that a value has been written
        self.disruptor.writer_move.notify(usize::MAX);

        // We do this at the end to ensure that we're not worrying about wierd drop functions or
        // allocations happening during the critical path
        if (claimed_id as usize) < self.capacity as usize {
            forget(old_value);
        } else {
            drop(old_value);
        }
        Ok(())
    }

    pub fn send_batch(&mut self, mut data: Vec<T>) -> Result<(), SenderError> {
        let total_num = data.len() as isize;
        let start_id = self.claim(data.len() as isize)?;
        let start_index = (start_id % self.capacity) as usize;
        let end_index = start_index + data.len();

        if end_index < self.capacity as usize {
            unsafe {
                let target = &mut (*self.disruptor.ring)[start_index..end_index];
                std::ptr::swap_nonoverlapping(data.as_mut_ptr(), target.as_mut_ptr(), data.len());
            }
            if start_id < self.capacity {
                forget(data.into_boxed_slice());
            }
        } else {
            let mut index = start_index;
            for value in data {
                if index < self.capacity as usize {
                    //forget the old value
                    unsafe {
                        let old = std::mem::replace(
                            &mut (*self.disruptor.ring)[index % self.capacity as usize],
                            value,
                        );
                        forget(old);
                    }
                } else {
                    //drop the old value
                    unsafe {
                        (*self.disruptor.ring)[index % self.capacity as usize] = value;
                    }
                }
                index += 1;
            }
        }

        while self
            .disruptor
            .committed
            .compare_exchange_weak(
                start_id - 1,
                start_id + total_num - 1,
                Ordering::Release,
                Ordering::Relaxed,
            )
            .is_err()
        {}
        // Notify other threads that a value has been written
        self.disruptor.writer_move.notify(usize::MAX);

        Ok(())
    }

    pub async fn async_send_batch(&mut self, mut data: Vec<T>) -> Result<(), SenderError> {
        let total_num = data.len() as isize;
        let start_id = self.async_claim(data.len() as isize).await?;
        let start_index = (start_id % self.capacity) as usize;
        let end_index = start_index + data.len();

        if end_index < self.capacity as usize {
            unsafe {
                let target = &mut (*self.disruptor.ring)[start_index..end_index];
                std::ptr::swap_nonoverlapping(data.as_mut_ptr(), target.as_mut_ptr(), data.len());
            }
            if start_id < self.capacity {
                forget(data.into_boxed_slice());
            }
        } else {
            let mut index = start_index;
            for value in data {
                if index < self.capacity as usize {
                    //forget the old value
                    unsafe {
                        let old = std::mem::replace(
                            &mut (*self.disruptor.ring)[index % self.capacity as usize],
                            value,
                        );
                        forget(old);
                    }
                } else {
                    //drop the old value
                    unsafe {
                        (*self.disruptor.ring)[index % self.capacity as usize] = value;
                    }
                }
                index += 1;
            }
        }

        while self
            .disruptor
            .committed
            .compare_exchange_weak(
                start_id - 1,
                start_id + total_num - 1,
                Ordering::Release,
                Ordering::Relaxed,
            )
            .is_err()
        {}
        // Notify other threads that a value has been written
        self.disruptor.writer_move.notify(usize::MAX);

        Ok(())
    }
}

#[cfg(test)]
mod sender_tests {
    use crate::channel::*;
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
        sender.send(0).expect("couldn't send");
        sender.send(1).expect("couldn't send");
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
