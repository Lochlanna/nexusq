use core::sync::atomic::{AtomicIsize, AtomicUsize, Ordering};

pub trait Waitable: Sync {
    type InnerType;
    fn greater_than_equal_to(&self, expected: &Self::InnerType) -> Option<Self::InnerType>;
    fn equal_to(&self, expected: &Self::InnerType) -> Option<Self::InnerType>;
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
    #[inline(always)]
    fn equal_to(&self, expected: &Self::InnerType) -> Option<Self::InnerType> {
        let value = self.load(Ordering::Acquire);
        if value == *expected {
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
    #[inline(always)]
    fn equal_to(&self, expected: &Self::InnerType) -> Option<Self::InnerType> {
        let value = self.load(Ordering::Acquire);
        if value == *expected {
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
    #[inline(always)]
    fn equal_to(&self, expected: &Self::InnerType) -> Option<Self::InnerType> {
        let value = self.load(Ordering::Acquire);
        if value == *expected {
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
    #[inline(always)]
    fn equal_to(&self, expected: &Self::InnerType) -> Option<Self::InnerType> {
        let value = self.load(Ordering::Acquire);
        if value == *expected {
            return Some(value);
        }
        None
    }
}

pub trait WaitStrategyBase: Clone {
    fn wait<V: Waitable>(
        &self,
        value: V,
        expected: V::InnerType,
        check: impl Fn(&V, &V::InnerType) -> Option<V::InnerType>,
    ) -> V::InnerType;
}

pub trait WaitStrategy: WaitStrategyBase {
    fn wait_for_geq<V: Waitable>(&self, value: V, expected: V::InnerType) -> V::InnerType {
        self.wait(value, expected, V::greater_than_equal_to)
    }
    fn wait_for_eq<V: Waitable>(&self, value: V, expected: V::InnerType) -> V::InnerType {
        self.wait(value, expected, V::equal_to)
    }
    #[inline(always)]
    fn notify(&self) {}
}

/// This is a raw spin loop. Super responsive. If you've got enough cores
#[derive(Debug, Clone, Default)]
pub struct BusyWait {}

impl WaitStrategyBase for BusyWait {
    fn wait<V: Waitable>(
        &self,
        value: V,
        expected: V::InnerType,
        check: impl Fn(&V, &V::InnerType) -> Option<V::InnerType>,
    ) -> V::InnerType {
        loop {
            if let Some(result) = check(&value, &expected) {
                return result;
            }
            core::hint::spin_loop();
        }
    }
}

impl WaitStrategy for BusyWait {}

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

impl WaitStrategyBase for YieldWait {
    fn wait<V: Waitable>(
        &self,
        value: V,
        expected: V::InnerType,
        check: impl Fn(&V, &V::InnerType) -> Option<V::InnerType>,
    ) -> V::InnerType {
        for _ in 0..self.num_spins {
            if let Some(result) = check(&value, &expected) {
                return result;
            }
            core::hint::spin_loop();
        }
        loop {
            if let Some(result) = check(&value, &expected) {
                return result;
            }
            std::thread::yield_now()
        }
    }
}

impl Default for YieldWait {
    fn default() -> Self {
        Self::new(100)
    }
}

impl WaitStrategy for YieldWait {}

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

impl WaitStrategyBase for SleepWait {
    fn wait<V: Waitable>(
        &self,
        value: V,
        expected: V::InnerType,
        check: impl Fn(&V, &V::InnerType) -> Option<V::InnerType>,
    ) -> V::InnerType {
        for _ in 0..self.num_spin {
            if let Some(result) = check(&value, &expected) {
                return result;
            }
            core::hint::spin_loop();
        }
        for _ in 0..self.num_yield {
            if let Some(result) = check(&value, &expected) {
                return result;
            }
            std::thread::yield_now();
        }
        loop {
            if let Some(result) = check(&value, &expected) {
                return result;
            }
            std::thread::park_timeout(self.sleep_time_ns);
        }
    }
}

impl Default for SleepWait {
    fn default() -> Self {
        Self::new(std::time::Duration::from_nanos(100), 100, 100)
    }
}

impl WaitStrategy for SleepWait {}

#[derive(Debug)]
pub struct SpinBlockWait {
    event: event_listener::Event,
    num_spin: u32,
    num_yield: u32,
}

impl Clone for SpinBlockWait {
    fn clone(&self) -> Self {
        Self::new(self.num_spin, self.num_yield)
    }
}

impl Default for SpinBlockWait {
    fn default() -> Self {
        Self::new(50, 50)
    }
}

impl SpinBlockWait {
    pub fn new(num_spin: u32, num_yield: u32) -> Self {
        Self {
            event: Default::default(),
            num_spin,
            num_yield,
        }
    }
}
impl WaitStrategyBase for SpinBlockWait {
    fn wait<V: Waitable>(
        &self,
        value: V,
        expected: V::InnerType,
        check: impl Fn(&V, &V::InnerType) -> Option<V::InnerType>,
    ) -> V::InnerType {
        for _ in 0..self.num_spin {
            if let Some(result) = check(&value, &expected) {
                return result;
            }
            core::hint::spin_loop();
        }
        for _ in 0..self.num_yield {
            if let Some(result) = check(&value, &expected) {
                return result;
            }
            std::thread::yield_now();
        }
        loop {
            if let Some(result) = check(&value, &expected) {
                return result;
            }
            let listener = self.event.listen();
            if let Some(result) = check(&value, &expected) {
                return result;
            }
            listener.wait();
        }
    }
}
impl WaitStrategy for SpinBlockWait {
    #[inline(always)]
    fn notify(&self) {
        self.event.notify(usize::MAX);
    }
}

#[derive(Debug, Default)]
pub struct BlockWait {
    event: event_listener::Event,
}

impl Clone for BlockWait {
    fn clone(&self) -> Self {
        Default::default()
    }
}

impl WaitStrategyBase for BlockWait {
    fn wait<V: Waitable>(
        &self,
        value: V,
        expected: V::InnerType,
        check: impl Fn(&V, &V::InnerType) -> Option<V::InnerType>,
    ) -> V::InnerType {
        loop {
            if let Some(result) = check(&value, &expected) {
                return result;
            }
            let listener = self.event.listen();
            if let Some(result) = check(&value, &expected) {
                return result;
            }
            listener.wait();
        }
    }
}

impl WaitStrategy for BlockWait {
    #[inline(always)]
    fn notify(&self) {
        self.event.notify(usize::MAX);
    }
}
