use super::*;
use std::sync::atomic::Ordering as AOrdering;
use std::sync::Arc;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SenderError {}

#[derive(Debug)]
pub struct Sender<T> {
    disruptor: Arc<DisruptorCore<T>>,
}

impl<T> From<Arc<DisruptorCore<T>>> for Sender<T> {
    fn from(disruptor: Arc<DisruptorCore<T>>) -> Self {
        Self { disruptor }
    }
}

impl<T> Sender<T> {
    //TODO this can probably be cut down / optimised a bit...
    fn claim(&self) -> isize {
        let claimed = self.disruptor.claimed.fetch_add(1, AOrdering::Release);
        //TODO we create this every time even though we hopefully wont use it every time

        let mut oldest_reader_id;
        if let Some(oldest) = self.disruptor.readers.front() {
            oldest_reader_id = oldest.value().current_id;
        } else {
            return claimed;
        }

        let capacity = self.disruptor.capacity as isize;
        while oldest_reader_id == claimed - capacity {
            let listener = self.disruptor.reader_move.listen();
            if let Some(oldest) = self.disruptor.readers.front() {
                oldest_reader_id = oldest.value().current_id;
            } else {
                return claimed;
            }
            if oldest_reader_id == claimed - capacity {
                return claimed;
            }
            listener.wait();
        }

        // TODO check if there is another writer writing to a different ID but the same cell.

        claimed
    }
    pub fn send(&self, value: T) {
        let claimed_id = self.claim();
        let index = claimed_id as usize % self.disruptor.capacity;
        unsafe {
            (*self.disruptor.ring)[index] = value;
        }
        // TODO what's the optimisation here
        // wait for writers to catch up and commit their transactions
        while self
            .disruptor
            .committed
            .compare_exchange_weak(
                claimed_id - 1,
                claimed_id,
                AOrdering::Release,
                AOrdering::Relaxed,
            )
            .is_err()
        {}
        // Notify other threads that a value has been written
        self.disruptor.writer_move.notify(usize::MAX);
    }
}
