#![allow(dead_code)]

#[macro_use]
extern crate failure;
extern crate hex;
extern crate nimiq_hash as hash;
extern crate nimiq_utils as utils;

use algebra::BigInteger768;

use hash::Blake2sHash;
pub use types::*;

// Implements big-endian compression
pub mod compression;

// Implements several serialization-related types
#[cfg(feature = "beserial")]
pub mod serialization;

// Implements the LazyPublicKey type. Which is a faster, cached version of PublicKey.
#[cfg(feature = "lazy")]
pub mod lazy;

// Implements all of the types needed to do BLS signatures.
mod types;

// Specifies the hash algorithm used for signatures
pub type SigHash = Blake2sHash;

pub fn big_int_from_bytes_be<R: std::io::Read>(reader: &mut R) -> BigInteger768 {
    let mut res = [0u64; 12];
    for num in res.iter_mut().rev() {
        let mut bytes = [0u8; 8];
        reader.read_exact(&mut bytes).unwrap();
        *num = u64::from_be_bytes(bytes);
    }
    BigInteger768::new(res)
}
