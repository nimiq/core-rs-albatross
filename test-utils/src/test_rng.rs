use rand::{self, rngs::StdRng, thread_rng, SeedableRng};

const TEST_RNG_SEED: [u8; 32] = [
    1, 0, 12, 0, 0, 6, 3, 0, 1, 0, 10, 0, 22, 32, 7, 3, 2, 0, 54, 6, 12, 11, 0, 0, 3, 0, 0, 0, 0,
    0, 2, 92,
];

pub fn test_rng(is_deterministic: bool) -> StdRng {
    // Allow to change setting via environment variable.
    let is_deterministic = if let Ok(val) = std::env::var("DETERMINISTIC_TEST_RNG") {
        val == "1"
    } else {
        is_deterministic
    };

    if is_deterministic {
        StdRng::from_seed(TEST_RNG_SEED)
    } else {
        StdRng::from_rng(thread_rng()).expect("Could not initialize test rng")
    }
}
