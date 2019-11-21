extern crate byteorder;

#[macro_use]
extern crate beserial_derive;
extern crate nimiq_hash as hash;
extern crate nimiq_bls as bls;

use std::io::Write;
use std::fmt;

use byteorder::{WriteBytesExt, ReadBytesExt, BigEndian};

use hash::{Blake2bHash, Blake2bHasher, Hasher, HashOutput};
use beserial::{Serialize, Deserialize};
use bls::bls12_381::{CompressedSignature, PublicKey, SecretKey};


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


#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct VrfSeed {
    signature: CompressedSignature,
}

impl VrfSeed {
    pub fn verify(&self, prev_seed: &VrfSeed, public_key: &PublicKey) -> Result<(), VrfError> {
        let signature = self.signature.uncompress()
            .map_err(|_| VrfError::InvalidSignature)?;

        // Hash use-case prefix and signature
        let mut hasher = Blake2bHasher::new();
        hasher.write_u8(VrfUseCase::Seed as u8).unwrap();
        hasher.write(prev_seed.signature.as_ref()).unwrap();

        if !public_key.verify_hash(hasher.finish(), &signature) {
            return Err(VrfError::Forged);
        }
        Ok(())
    }

    pub fn sign_next(&self, secret_key: &SecretKey) -> Self {
        // Hash use-case prefix and signature
        let mut hasher = Blake2bHasher::new();
        hasher.write_u8(VrfUseCase::Seed as u8).unwrap();
        hasher.write(self.signature.as_ref()).unwrap();

        // Sign that hash and contruct new VrfSeed from it
        let signature = secret_key
            .sign_hash(hasher.finish())
            .compress();
        Self {
            signature
        }
    }

    pub fn rng<'s>(&'s self, use_case: VrfUseCase) -> VrfRng<'s> {
        VrfRng::new(&self.signature, use_case)
    }
}

impl From<CompressedSignature> for VrfSeed {
    fn from(signature: CompressedSignature) -> Self {
        Self {
            signature
        }
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
    counter: u64,
}

impl<'s> VrfRng<'s> {
    fn new(signature: &'s CompressedSignature, use_case: VrfUseCase) -> Self {
        Self {
            signature,
            use_case,
            counter: 0,
        }
    }

    pub fn next_hash(&mut self) -> Blake2bHash {
        // Hash use-case prefix, counter and signature
        let mut hasher = Blake2bHasher::new();
        hasher.write_u8(self.use_case as u8).unwrap();
        hasher.write_u64::<BigEndian>(self.counter).unwrap();
        hasher.write(self.signature.as_ref()).unwrap();

        // Increase counter
        self.counter += 1;

        hasher.finish()
    }

    pub fn next_u64(&mut self) -> u64 {
        self.next_hash()
            .as_bytes()
            .read_u64::<BigEndian>()
            .unwrap()
    }

    pub fn next_u64_max(&mut self, max: u64) -> u64 {
        let bitmask = max.next_power_of_two() - 1;

        loop {
            let x = self.next_u64() & bitmask;
            if x < max {
                break x;
            }
        }
    }
}
