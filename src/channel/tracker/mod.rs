pub mod broadcast_tracker;

use event_listener::Event;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

const UNUSED: usize = (isize::MAX as usize) + 1;

#[derive(Debug)]
pub(crate) struct ObservableCell {
    event: Event,
    value: AtomicUsize,
}

impl ObservableCell {
    #[inline(always)]
    pub fn set_unused(&self) {
        self.value.store(UNUSED, Ordering::Release);
    }
    #[inline(always)]
    pub fn store(&self, value: usize) {
        self.value.store(value, Ordering::Release);
        self.event.notify(usize::MAX);
    }
    #[inline(always)]
    pub fn is_unused(&self) -> bool {
        self.value.load(Ordering::Acquire) == UNUSED
    }
    #[inline(always)]
    pub fn wait_for_completion(&self, value: usize) {
        let target = value + 1;
        loop {
            if self.value.load(Ordering::Acquire) >= target {
                return;
            }
            let listener = self.event.listen();
            if self.value.load(Ordering::Acquire) >= target {
                return;
            }
            listener.wait();
        }
    }

    #[inline(always)]
    pub async fn async_wait_for_completion(&self, value: usize) {
        let target = value + 1;
        loop {
            if self.value.load(Ordering::Acquire) >= target {
                return;
            }
            let listener = self.event.listen();
            if self.value.load(Ordering::Acquire) >= target {
                return;
            }
            listener.await;
        }
    }

    pub fn load(&self, order: Ordering) -> usize {
        self.value.load(order)
    }

    pub fn at(value: usize) -> Self {
        Self {
            event: Default::default(),
            value: AtomicUsize::new(value),
        }
    }
}

impl Default for ObservableCell {
    fn default() -> Self {
        Self {
            event: Default::default(),
            value: Default::default(),
        }
    }
}

pub(crate) trait Tracker {
    fn new_receiver(&self, at: isize) -> Arc<ObservableCell>;
    //TODO better name for cell?
    fn remove_receiver(&self, cell: Arc<ObservableCell>);
    fn slowest(&self, min: isize) -> (isize, Option<Arc<ObservableCell>>);
    /// Runs the tracker garbage collection
    fn tidy(&self);
    fn garbage_count(&self) -> usize;
}
