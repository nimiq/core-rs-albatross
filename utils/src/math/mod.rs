mod exp;

pub use exp::exp;

pub trait CeilingDiv {
    #[must_use]
    fn ceiling_div(self, rhs: Self) -> Self;
}

// Fast integer division with ceiling: (x + y - 1) / y
macro_rules! ceiling_div {
    ($t: ty) => {
        impl CeilingDiv for $t {
            #[inline]
            fn ceiling_div(self, rhs: Self) -> Self {
                (self + rhs - 1) / rhs
            }
        }
    };
}

ceiling_div!(u8);
ceiling_div!(u16);
ceiling_div!(u32);
ceiling_div!(u64);
ceiling_div!(usize);
