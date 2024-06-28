/// Computes `x` to the power of `n` using exponentiation by squaring.
///
/// The algorithm is adapted from
/// <https://en.wikipedia.org/w/index.php?title=Exponentiation_by_squaring&oldid=1229001691#With_constant_auxiliary_memory>,
/// removing the negative case:
///
/// ```text
///   Function exp_by_squaring_iterative(x, n)
///    if n < 0 then
///      x := 1 / x;
///      n := -n;
///    if n = 0 then return 1
///    y := 1;
///    while n > 1 do
///      if n is odd then
///        y := x * y;
///        n := n - 1;
///      x := x * x;
///      n := n / 2;
///    return x * y
/// ```
///
/// On IEEE 754 compliant systems, this always gives the same results given the
/// same inputs.
pub fn powi(mut x: f64, mut n: u64) -> f64 {
    if n == 0 {
        return 1.0;
    }
    // y * x**n is invariant in this loop
    let mut y = 1.0;
    while n > 1 {
        if n % 2 == 1 {
            y *= x;
            n -= 1;
        }
        x *= x;
        n /= 2;
    }
    x * y
}

#[cfg(test)]
mod test {
    use super::powi;

    #[test]
    fn correctness() {
        for i in 0..64 {
            assert_eq!(powi(2.0, i), (1u64 << i) as f64);
        }
        assert_eq!(powi(2.0_f64.sqrt(), 2), 2.0000000000000004);
    }
}
