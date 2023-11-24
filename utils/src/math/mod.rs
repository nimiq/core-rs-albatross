mod exp;

pub use exp::exp;

const fn num_bits<T>() -> usize {
    std::mem::size_of::<T>() * 8
}

pub fn log2(x: usize) -> usize {
    if x == 0 {
        panic!("log_2(0) is undefined.");
    }
    num_bits::<usize>() - (x.leading_zeros() as usize) - 1
}

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

#[cfg(test)]
mod tests {
    use nimiq_test_log::test;

    use super::log2;

    #[test]
    fn test_log2() {
        assert_eq!(log2(1), 0);
        assert_eq!(log2(2), 1);
        assert_eq!(log2(4), 2);
        assert_eq!(log2(8), 3);
        assert_eq!(log2(16), 4);
        assert_eq!(log2(32), 5);
    }
}
