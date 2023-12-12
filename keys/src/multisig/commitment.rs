use curve25519_dalek::{
    constants, edwards::CompressedEdwardsY, traits::Identity, EdwardsPoint, Scalar,
};
use nimiq_utils::key_rng::SecureGenerate;
use rand::Rng;
use rand_core::{CryptoRng, RngCore};
use sha2::digest::Update;
use zeroize::Zeroize;

use super::{error::InvalidScalarError, MUSIG2_PARAMETER_V};

#[derive(PartialEq, Eq, Debug, Clone, Copy, Zeroize)]
pub struct Nonce(pub Scalar);

impl Nonce {
    pub const SIZE: usize = 32;

    pub fn commit(&self) -> Commitment {
        // Compute the point [scalar]B.
        let commitment: EdwardsPoint = &self.0 * constants::ED25519_BASEPOINT_TABLE;
        Commitment(commitment)
    }
}

impl From<[u8; Nonce::SIZE]> for Nonce {
    fn from(bytes: [u8; Nonce::SIZE]) -> Self {
        Nonce(Scalar::from_bytes_mod_order(bytes))
    }
}

impl<'a> From<&'a [u8; Nonce::SIZE]> for Nonce {
    fn from(bytes: &'a [u8; Nonce::SIZE]) -> Self {
        Nonce::from(*bytes)
    }
}

#[derive(PartialEq, Eq, Debug, Clone, Copy, Default)]
pub struct Commitment(pub EdwardsPoint);
implement_simple_add_sum_traits!(Commitment, EdwardsPoint::identity());

impl Commitment {
    pub const SIZE: usize = 32;

    #[inline]
    pub fn to_bytes(&self) -> [u8; Commitment::SIZE] {
        self.0.compress().to_bytes()
    }

    pub fn from_bytes(bytes: [u8; Commitment::SIZE]) -> Option<Self> {
        let compressed = CompressedEdwardsY(bytes);
        compressed.decompress().map(Commitment)
    }
}

impl From<[u8; Commitment::SIZE]> for Commitment {
    fn from(bytes: [u8; Commitment::SIZE]) -> Self {
        Commitment::from_bytes(bytes).unwrap()
    }
}

impl<'a> From<&'a [u8; Commitment::SIZE]> for Commitment {
    fn from(bytes: &'a [u8; Commitment::SIZE]) -> Self {
        Commitment::from(*bytes)
    }
}

#[derive(PartialEq, Eq, Debug, Clone, Copy)]
#[cfg_attr(feature = "serde-derive", derive(serde::Serialize, serde::Deserialize))]
pub struct CommitmentPair {
    random_secret: Nonce,
    commitment: Commitment,
}

impl CommitmentPair {
    pub fn new(random_secret: Nonce, commitment: Commitment) -> Self {
        CommitmentPair {
            random_secret,
            commitment,
        }
    }

    pub fn generate_all<R: Rng + RngCore + CryptoRng>(
        rng: &mut R,
    ) -> [CommitmentPair; MUSIG2_PARAMETER_V] {
        let mut commitments = Vec::with_capacity(MUSIG2_PARAMETER_V);
        for _ in 0..MUSIG2_PARAMETER_V {
            commitments.push(CommitmentPair::generate(rng));
        }
        commitments.try_into().unwrap()
    }

    fn generate_internal<R: Rng + RngCore + CryptoRng>(
        rng: &mut R,
    ) -> Result<CommitmentPair, InvalidScalarError> {
        // Create random 32 bytes.
        let mut randomness: [u8; Nonce::SIZE] = [0u8; Nonce::SIZE];
        rng.fill(&mut randomness);

        // Decompress the 32 byte cryptographically secure random data to 64 byte.
        let mut h: sha2::Sha512 = sha2::Sha512::default();

        h.update(&randomness);
        let scalar = Scalar::from_hash::<sha2::Sha512>(h);
        if scalar == Scalar::ZERO || scalar == Scalar::ONE {
            return Err(InvalidScalarError);
        }

        let rs = Nonce(scalar);
        let ct = rs.commit();

        Ok(CommitmentPair {
            random_secret: rs,
            commitment: ct,
        })
    }

    #[inline]
    pub fn nonce(&self) -> Nonce {
        self.random_secret
    }

    #[inline]
    pub fn commitment(&self) -> Commitment {
        self.commitment
    }

    pub fn to_commitments(
        value: &[CommitmentPair; MUSIG2_PARAMETER_V],
    ) -> [Commitment; MUSIG2_PARAMETER_V] {
        let mut res = [Commitment::default(); MUSIG2_PARAMETER_V];
        for i in 0..MUSIG2_PARAMETER_V {
            res[i] = value[i].commitment();
        }
        res
    }
}

impl SecureGenerate for CommitmentPair {
    fn generate<R: Rng + RngCore + CryptoRng>(rng: &mut R) -> Self {
        CommitmentPair::generate_internal(rng).expect("Failed to generate CommitmentPair")
    }
}

#[cfg(feature = "serde-derive")]
mod serde_derive {
    use serde::{
        de::{Deserialize, Deserializer, Error},
        ser::{Serialize, Serializer},
    };

    use super::{Commitment, Nonce};

    impl Serialize for Commitment {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            serde_big_array::BigArray::serialize(&self.to_bytes(), serializer)
        }
    }

    impl<'de> Deserialize<'de> for Commitment {
        fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where
            D: Deserializer<'de>,
        {
            let buf: [u8; Commitment::SIZE] = serde_big_array::BigArray::deserialize(deserializer)?;
            Self::from_bytes(buf).ok_or_else(|| D::Error::custom("Invalid commitment"))
        }
    }

    impl Serialize for Nonce {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            serde_big_array::BigArray::serialize(&self.0.to_bytes(), serializer)
        }
    }

    impl<'de> Deserialize<'de> for Nonce {
        fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where
            D: Deserializer<'de>,
        {
            let buf: [u8; Nonce::SIZE] = serde_big_array::BigArray::deserialize(deserializer)?;
            Ok(Self::from(&buf))
        }
    }
}
