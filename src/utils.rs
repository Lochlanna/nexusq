pub trait FastMod: Sized {
    fn pow_2_mod(&self, denominator: Self) -> Self;
}
impl FastMod for isize {
    #[inline(always)]
    fn pow_2_mod(&self, denominator: Self) -> Self {
        debug_assert!(*self >= 0);
        debug_assert!(denominator.is_positive());
        debug_assert!((denominator as usize).is_power_of_two());
        *self & (denominator - 1)
    }
}

impl FastMod for usize {
    #[inline(always)]
    fn pow_2_mod(&self, denominator: Self) -> Self {
        debug_assert!(denominator.is_power_of_two());
        *self & (denominator - 1)
    }
}
