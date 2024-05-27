use ark_ff::{UniformRand, Zero};
use ark_mnt6_753::{Fr, G1Projective};
use nimiq_hash::Hash;
use nimiq_utils::key_rng::{CryptoRng, RngCore, SecureGenerate};

use crate::{CompressedSignature, SigHash, Signature};

#[derive(Clone, Copy)]
pub struct SecretKey {
    /// This is simply a number in the finite field Fr.
    /// Fr is also the prime subgroup of the elliptic curve.
    pub secret_key: Fr,
}

impl SecretKey {
    pub const SIZE: usize = 95;

    /// Creates a signature given a message.
    pub fn sign<M: Hash>(&self, msg: &M) -> Signature {
        self.sign_hash(msg.hash())
    }

    /// Creates a signature given a hash.
    pub fn sign_hash(&self, hash: SigHash) -> Signature {
        self.sign_g1(Signature::hash_to_g1(hash))
    }

    /// Creates a signature given a G1 point.
    pub fn sign_g1(&self, hash_curve: G1Projective) -> Signature {
        let mut sig = hash_curve;
        sig *= self.secret_key;
        Signature {
            signature: sig,
            compressed: CompressedSignature::from(sig),
        }
    }
}

impl SecureGenerate for SecretKey {
    fn generate<R: RngCore + CryptoRng>(rng: &mut R) -> Self {
        let mut x = Fr::rand(rng);
        loop {
            if !x.is_zero() {
                break;
            }
            x = Fr::rand(rng);
        }
        SecretKey { secret_key: x }
    }
}

impl Eq for SecretKey {}

impl PartialEq for SecretKey {
    fn eq(&self, other: &Self) -> bool {
        self.secret_key.eq(&other.secret_key)
    }
}

#[cfg(feature = "serde-derive")]
mod serde_derive {
    // TODO: Replace this with a generic serialization using `ToHex` and `FromHex`.
    use std::fmt;

    use ark_mnt6_753::Fr;
    use ark_serialize::{CanonicalDeserialize, CanonicalSerialize};
    use nimiq_serde::Serialize as NimiqSerialize;
    use serde::{
        de::{Deserialize, Deserializer, Error as SerializationError},
        ser::{Error as DeSerializationError, Serialize, Serializer},
    };

    use super::SecretKey;

    impl fmt::Display for SecretKey {
        fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
            write!(f, "{}", hex::encode(self.serialize_to_vec()))
        }
    }

    impl fmt::Debug for SecretKey {
        fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
            write!(f, "SecretKey({})", hex::encode(self.serialize_to_vec()))
        }
    }

    impl Serialize for SecretKey {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            let mut compressed = [0u8; Self::SIZE];
            self.secret_key
                .serialize_uncompressed(&mut compressed[..])
                .map_err(|_| S::Error::custom("Couldn't compress secret key"))?;
            Serialize::serialize(
                &nimiq_serde::FixedSizeByteArray::from(compressed),
                serializer,
            )
        }
    }

    impl<'de> Deserialize<'de> for SecretKey {
        fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where
            D: Deserializer<'de>,
        {
            let compressed: [u8; Self::SIZE] =
                nimiq_serde::FixedSizeByteArray::deserialize(deserializer)?.into_inner();
            Ok(SecretKey {
                secret_key: Fr::deserialize_uncompressed(&*compressed.to_vec())
                    .map_err(|_| D::Error::custom("Couldn't uncompress secret key"))?,
            })
        }
    }
}
