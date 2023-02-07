use core::sync::atomic::{AtomicIsize, AtomicUsize, Ordering};

pub trait Waitable: Sync {
    type InnerType;
    fn greater_than_equal_to(&self, expected: &Self::InnerType) -> Option<Self::InnerType>;
}

impl Waitable for AtomicIsize {
    type InnerType = isize;
    #[inline(always)]
    fn greater_than_equal_to(&self, expected: &Self::InnerType) -> Option<Self::InnerType> {
        let value = self.load(Ordering::Acquire);
        if value >= *expected {
            return Some(value);
        }
        None
    }
}

impl Waitable for AtomicUsize {
    type InnerType = usize;

    #[inline(always)]
    fn greater_than_equal_to(&self, expected: &Self::InnerType) -> Option<Self::InnerType> {
        let value = self.load(Ordering::Acquire);
        if value >= *expected {
            return Some(value);
        }
        None
    }
}

impl Waitable for &AtomicIsize {
    type InnerType = isize;
    #[inline(always)]
    fn greater_than_equal_to(&self, expected: &Self::InnerType) -> Option<Self::InnerType> {
        let value = self.load(Ordering::Acquire);
        if value >= *expected {
            return Some(value);
        }
        None
    }
}

impl Waitable for &AtomicUsize {
    type InnerType = usize;

    #[inline(always)]
    fn greater_than_equal_to(&self, expected: &Self::InnerType) -> Option<Self::InnerType> {
        let value = self.load(Ordering::Acquire);
        if value >= *expected {
            return Some(value);
        }
        None
    }
}

pub trait WaitStrategy {
    fn wait_for<V: Waitable>(&self, value: V, expected: V::InnerType) -> V::InnerType;
    fn notify(&self) {}
}

/// This is a raw spin loop. Super responsive. If you've got enough cores
#[derive(Debug, Clone, Default)]
pub struct BusyWait {}

impl WaitStrategy for BusyWait {
    fn wait_for<V: Waitable>(&self, value: V, expected: V::InnerType) -> V::InnerType {
        loop {
            if let Some(result) = value.greater_than_equal_to(&expected) {
                return result;
            }
            core::hint::spin_loop();
        }
    }
}

/// This is a yield loop. decently responsive.
/// Will let other things progress but still has high cpu usage
#[derive(Debug, Clone, Default)]
pub struct YieldWait {}

impl YieldWait {
    const SPIN_TRIES: u32 = 100;
}

impl WaitStrategy for YieldWait {
    fn wait_for<V: Waitable>(&self, value: V, expected: V::InnerType) -> V::InnerType {
        let mut counter = Self::SPIN_TRIES;
        loop {
            if let Some(result) = value.greater_than_equal_to(&expected) {
                return result;
            }
            if counter == 0 {
                std::thread::yield_now()
            } else {
                counter -= 1;
                std::hint::spin_loop();
            }
        }
    }
}

/// This is a raw spin loop. Super responsive. If you've got enough cores
#[derive(Debug, Clone)]
pub struct SleepWait {
    sleep_time_ns: std::time::Duration,
    num_retries: u32,
}

impl SleepWait {
    const SPIN_THRESHOLD: u32 = 100;
    pub fn new(sleep_time_ns: std::time::Duration, num_retries: u32) -> Self {
        Self {
            sleep_time_ns,
            num_retries,
        }
    }
}

impl Default for SleepWait {
    fn default() -> Self {
        Self {
            sleep_time_ns: std::time::Duration::from_nanos(100),
            num_retries: 200,
        }
    }
}

impl WaitStrategy for SleepWait {
    fn wait_for<V: Waitable>(&self, value: V, expected: V::InnerType) -> V::InnerType {
        let mut counter = self.num_retries;
        loop {
            if let Some(result) = value.greater_than_equal_to(&expected) {
                return result;
            }
            if counter > Self::SPIN_THRESHOLD {
                counter -= 1;
                core::hint::spin_loop();
            } else if counter > 0 {
                std::thread::yield_now();
                counter -= 1;
            } else {
                std::thread::park_timeout(self.sleep_time_ns);
            }
        }
    }
}

#[derive(Debug)]
pub struct BlockWait {
    event: event_listener::Event,
    num_retries: u32,
}

impl Clone for BlockWait {
    fn clone(&self) -> Self {
        Self {
            event: Default::default(),
            num_retries: self.num_retries,
        }
    }
}

impl Default for BlockWait {
    fn default() -> Self {
        Self {
            event: event_listener::Event::default(),
            num_retries: Self::DEFAULT_NUM_RETRIES,
        }
    }
}

impl BlockWait {
    const DEFAULT_NUM_RETRIES: u32 = 100;
    const SPIN_THRESHOLD: u32 = 100;
    pub fn new(num_retries: u32) -> Self {
        Self {
            event: event_listener::Event::default(),
            num_retries,
        }
    }
}

impl WaitStrategy for BlockWait {
    fn wait_for<V: Waitable>(&self, value: V, expected: V::InnerType) -> V::InnerType {
        let mut counter = self.num_retries;
        loop {
            if let Some(result) = value.greater_than_equal_to(&expected) {
                return result;
            }
            if counter > Self::SPIN_THRESHOLD {
                counter -= 1;
                core::hint::spin_loop();
            } else if counter > 0 {
                std::thread::yield_now();
                counter -= 1;
            } else {
                let listener = self.event.listen();
                if let Some(result) = value.greater_than_equal_to(&expected) {
                    return result;
                }
                listener.wait();
            }
        }
    }
    fn notify(&self) {
        self.event.notify(usize::MAX);
    }
}
