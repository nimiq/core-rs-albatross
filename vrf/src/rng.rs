pub trait Rng {
    /// Returns a random 64-bit integer
    ///
    /// It is chosen from a uniform distribution.
    fn next_u64(&mut self) -> u64;

    /// Returns a random integer in range [0, limit)
    ///
    /// It's chosen from a uniform distribution using rejection sampling.
    ///
    /// See <https://en.wikipedia.org/wiki/Rejection_sampling>.
    ///
    /// # Panics
    ///
    /// If `limit` is zero, this function panics.
    fn next_u64_below(&mut self, limit: u64) -> u64 {
        assert!(limit != 0, "limit can't be 0");

        let bitmask = limit.next_power_of_two() - 1;

        loop {
            // Get a integer in the range [0, n] where n is the next power of 2
            let x = self.next_u64() & bitmask;

            // Resample until the integer is below the limit
            if x < limit {
                break x;
            }
        }
    }
}
