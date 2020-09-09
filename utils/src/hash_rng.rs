use std::default::Default;
use std::io::Write;
use std::marker::PhantomData;

use rand_core::{impls, Error, RngCore, SeedableRng};

use nimiq_hash::{HashOutput, Hasher};

pub struct HashRng<S: Sized + Default + AsMut<[u8]>, H: HashOutput> {
    seed: S,
    counter: u64,
    _hash: PhantomData<H>,
}

impl<S: Sized + Default + AsMut<[u8]>, H: HashOutput> HashRng<S, H> {
    fn next_hash(&mut self) -> H {
        let mut hash_state = H::Builder::default();
        hash_state
            .write_all(self.seed.as_mut())
            .expect("Failed to hash seed");
        hash_state
            .write_all(&self.counter.to_be_bytes())
            .expect("Failed to hash index");
        self.counter += 1;
        hash_state.finish()
    }
}

impl<S: Sized + Default + AsMut<[u8]>, H: HashOutput> RngCore for HashRng<S, H> {
    fn next_u32(&mut self) -> u32 {
        self.next_u64() as u32
    }

    fn next_u64(&mut self) -> u64 {
        let hash = self.next_hash();

        // Get number from first 8 bytes
        let mut num_bytes = [0u8; 8];
        num_bytes.copy_from_slice(&hash.as_bytes()[..8]);
        u64::from_be_bytes(num_bytes)
    }

    fn fill_bytes(&mut self, dest: &mut [u8]) {
        impls::fill_bytes_via_next(self, dest)
    }

    fn try_fill_bytes(&mut self, dest: &mut [u8]) -> Result<(), Error> {
        self.fill_bytes(dest);
Ok(())
    }
}

impl<S: Sized + Default + AsMut<[u8]>, H: HashOutput> SeedableRng for HashRng<S, H> {
    type Seed = S;

    fn from_seed(seed: Self::Seed) -> Self {
        HashRng {
            seed,
            counter: 0,
            _hash: PhantomData,
        }
    }
}
