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
    #[inline(always)]
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
#[derive(Debug, Clone)]
pub struct YieldWait {
    num_spins: u32,
}

impl YieldWait {
    pub fn new(num_spins: u32) -> Self {
        Self { num_spins }
    }
}

impl Default for YieldWait {
    fn default() -> Self {
        Self::new(100)
    }
}

impl WaitStrategy for YieldWait {
    fn wait_for<V: Waitable>(&self, value: V, expected: V::InnerType) -> V::InnerType {
        for _ in 0..self.num_spins {
            if let Some(result) = value.greater_than_equal_to(&expected) {
                return result;
            }
            core::hint::spin_loop();
        }
        loop {
            if let Some(result) = value.greater_than_equal_to(&expected) {
                return result;
            }
            std::thread::yield_now()
        }
    }
}

/// This is a raw spin loop. Super responsive. If you've got enough cores
#[derive(Debug, Clone)]
pub struct SleepWait {
    sleep_time_ns: std::time::Duration,
    num_spin: u32,
    num_yield: u32,
}

impl SleepWait {
    pub fn new(sleep_time_ns: std::time::Duration, num_spin: u32, num_yield: u32) -> Self {
        Self {
            sleep_time_ns,
            num_spin,
            num_yield,
        }
    }
}

impl Default for SleepWait {
    fn default() -> Self {
        Self::new(std::time::Duration::from_nanos(100), 100, 100)
    }
}

impl WaitStrategy for SleepWait {
    fn wait_for<V: Waitable>(&self, value: V, expected: V::InnerType) -> V::InnerType {
        for _ in 0..self.num_spin {
            if let Some(result) = value.greater_than_equal_to(&expected) {
                return result;
            }
            core::hint::spin_loop();
        }
        for _ in 0..self.num_yield {
            if let Some(result) = value.greater_than_equal_to(&expected) {
                return result;
            }
            std::thread::yield_now();
        }
        loop {
            if let Some(result) = value.greater_than_equal_to(&expected) {
                return result;
            }
            std::thread::park_timeout(self.sleep_time_ns);
        }
    }
}

#[derive(Debug)]
pub struct BlockWait {
    lock: parking_lot::Mutex<()>,
    con: parking_lot::Condvar,
    num_spin: u32,
    num_yield: u32,
}

impl Clone for BlockWait {
    fn clone(&self) -> Self {
        Self::new(self.num_spin, self.num_yield)
    }
}

impl Default for BlockWait {
    fn default() -> Self {
        Self::new(50, 50)
    }
}

impl BlockWait {
    pub fn new(num_spin: u32, num_yield: u32) -> Self {
        Self {
            lock: parking_lot::Mutex::default(),
            con: parking_lot::Condvar::default(),
            num_spin,
            num_yield,
        }
    }
}

impl WaitStrategy for BlockWait {
    fn wait_for<V: Waitable>(&self, value: V, expected: V::InnerType) -> V::InnerType {
        for _ in 0..self.num_spin {
            if let Some(result) = value.greater_than_equal_to(&expected) {
                return result;
            }
            core::hint::spin_loop();
        }
        for _ in 0..self.num_yield {
            if let Some(result) = value.greater_than_equal_to(&expected) {
                return result;
            }
            std::thread::yield_now();
        }
        loop {
            if let Some(result) = value.greater_than_equal_to(&expected) {
                return result;
            }
            let mut lock = self.lock.lock();
            if let Some(result) = value.greater_than_equal_to(&expected) {
                return result;
            }
            self.con.wait(&mut lock);
        }
    }
    #[inline(always)]
    fn notify(&self) {
        let _lock = self.lock.lock();
        self.con.notify_all();
    }
}
