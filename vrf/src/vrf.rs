use std::fmt;
use std::hash::{Hash, Hasher as StdHasher};
use std::io::Write;

use byteorder::{BigEndian, ReadBytesExt, WriteBytesExt};

use beserial::{Deserialize, Serialize};
use bls::{CompressedSignature, PublicKey, SecretKey};
use hash::{Blake2sHash, Blake2sHasher, Hasher};

use crate::rng::Rng;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum VrfError {
    Forged,
    InvalidSignature,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
#[repr(u8)]
pub enum VrfUseCase {
    Seed = 1,
    ValidatorSelection = 2,
    SlotSelection = 3,
    RewardDistribution = 4,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
pub struct VrfSeed {
    signature: CompressedSignature,
}

impl VrfSeed {
    pub fn verify(&self, prev_seed: &VrfSeed, public_key: &PublicKey) -> Result<(), VrfError> {
        let signature = self
            .signature
            .uncompress()
            .map_err(|_| VrfError::InvalidSignature)?;

        // Hash use-case prefix and signature
        let mut hasher = Blake2sHasher::new();
        hasher.write_u8(VrfUseCase::Seed as u8).unwrap();
        hasher.write_all(prev_seed.signature.as_ref()).unwrap();

        if !public_key.verify_hash(hasher.finish(), &signature) {
            return Err(VrfError::Forged);
        }
        Ok(())
    }

    pub fn sign_next(&self, secret_key: &SecretKey) -> Self {
        // Hash use-case prefix and signature
        let mut hasher = Blake2sHasher::new();
        hasher.write_u8(VrfUseCase::Seed as u8).unwrap();
        hasher.write_all(self.signature.as_ref()).unwrap();

        // Sign that hash and contruct new VrfSeed from it
        let signature = secret_key.sign_hash(hasher.finish()).compress();
        Self { signature }
    }

    pub fn rng(&self, use_case: VrfUseCase, round: u32) -> VrfRng {
        VrfRng::new(&self.signature, use_case, round)
    }
}

impl From<CompressedSignature> for VrfSeed {
    fn from(signature: CompressedSignature) -> Self {
        Self { signature }
    }
}

// Disable clippy error because the property "k1 == k2 -> hash(k1) == hash(k2)" is maintained here since PartialEq
// derivation is based on all fields being equal (see https://doc.rust-lang.org/std/cmp/trait.PartialEq.html#derivable)
// which implies that `self.signature.as_ref()` would be equal, and thus the hash value of it would be equal too
#[allow(clippy::derive_hash_xor_eq)]
impl Hash for VrfSeed {
    fn hash<H: StdHasher>(&self, state: &mut H) {
        self.signature.as_ref().hash(state)
    }
}

impl fmt::Display for VrfSeed {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        fmt::Display::fmt(&self.signature, f)
    }
}

pub struct VrfRng<'s> {
    signature: &'s CompressedSignature,
    use_case: VrfUseCase,
    round: u32,
    counter: u64,
}

impl<'s> VrfRng<'s> {
    fn new(signature: &'s CompressedSignature, use_case: VrfUseCase, round: u32) -> Self {
        Self {
            signature,
            use_case,
            round,
            counter: 0,
        }
    }

    pub fn next_hash(&mut self) -> Blake2sHash {
        // Hash use-case prefix, round, counter and signature
        let mut hasher = Blake2sHasher::new();
        hasher.write_u8(self.use_case as u8).unwrap();
        hasher.write_u32::<BigEndian>(self.round).unwrap();
        hasher.write_u64::<BigEndian>(self.counter).unwrap();
        hasher.write_all(self.signature.as_ref()).unwrap();

        // Increase counter
        self.counter += 1;

        hasher.finish()
    }
}

impl<'s> Rng for VrfRng<'s> {
    fn next_u64(&mut self) -> u64 {
        self.next_hash().as_bytes().read_u64::<BigEndian>().unwrap()
    }
}
