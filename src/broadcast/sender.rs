use super::*;
use std::mem::forget;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SenderError {}

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
    fn claim(&mut self) -> isize {
        let claimed = self.disruptor.claimed.fetch_add(1, Ordering::Release);
        //TODO we create this every time even though we hopefully wont use it every time

        let tail = claimed - self.capacity;

        if self.cached_slowest_reader != -1 && self.cached_slowest_reader > tail {
            return claimed;
        }

        let mut slowest_reader = self.disruptor.readers.slowest(tail);

        while slowest_reader <= tail {
            let listener = self.disruptor.reader_move.listen();
            // the reader may have moved before we managed to register the listener so
            // we need to check again before we wait. If a reader moves between now and when
            // we call wait it should return immediately
            slowest_reader = self.disruptor.readers.slowest(tail);
            if slowest_reader > tail {
                return claimed;
            }
            listener.wait();
            slowest_reader = self.disruptor.readers.slowest(tail);
        }
        // TODO check if there is another writer writing to a different ID but the same cell.
        if slowest_reader > tail {
            self.cached_slowest_reader = slowest_reader;
        }

        claimed
    }
    pub fn send(&mut self, value: T) {
        let claimed_id = self.claim();
        let index = claimed_id as usize % self.capacity as usize;

        let old_value;
        unsafe { old_value = std::mem::replace(&mut (*self.disruptor.ring)[index], value) }

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
    }
}
