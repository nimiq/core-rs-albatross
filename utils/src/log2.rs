
const fn num_bits<T>() -> usize {
    std::mem::size_of::<T>() * 8
}

pub fn log2(x: usize) -> usize {
    if x == 0 {
        panic!("log_2(0) is undefined.");
    }
    (num_bits::<usize>() as usize) - (x.leading_zeros() as usize) - 1
}


#[cfg(test)]
mod tests{
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
