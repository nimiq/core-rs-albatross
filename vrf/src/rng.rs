pub trait Rng {
    /// Returns a random 64 bit integer
    fn next_u64(&mut self) -> u64;

    /// Returns a random integer in range [0, max)
    fn next_u64_max(&mut self, max: u64) -> u64 {
        let bitmask = max.next_power_of_two() - 1;

        loop {
            // Get a integer in the range [0, n] where n is the next power of 2
            let x = self.next_u64() & bitmask;

            // Resample until the integer is below the max
            if x < max {
                break x;
            }
        }
    }
}
