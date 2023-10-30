use std::{borrow::Borrow, error, fmt, iter::Sum, ops::Add};

use curve25519_dalek::{
    constants,
    edwards::{CompressedEdwardsY, EdwardsPoint},
    scalar::Scalar,
    traits::Identity,
};
use nimiq_utils::key_rng::{CryptoRng, RngCore, SecureGenerate};
use rand::Rng;
use sha2::{self, Digest, Sha512};

use crate::{KeyPair, PublicKey, Signature};

#[derive(PartialEq, Eq, Debug, Clone, Copy)]
pub struct RandomSecret(pub Scalar);

impl RandomSecret {
    pub const SIZE: usize = 32;
}

impl From<[u8; RandomSecret::SIZE]> for RandomSecret {
    fn from(bytes: [u8; RandomSecret::SIZE]) -> Self {
        RandomSecret(Scalar::from_bytes_mod_order(bytes))
    }
}

impl<'a> From<&'a [u8; RandomSecret::SIZE]> for RandomSecret {
    fn from(bytes: &'a [u8; RandomSecret::SIZE]) -> Self {
        RandomSecret::from(*bytes)
    }
}

#[derive(PartialEq, Eq, Debug, Clone, Copy)]
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

#[derive(Debug, Clone, Copy)]
pub struct InvalidScalarError;

impl fmt::Display for InvalidScalarError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Generated scalar was invalid (0 or 1).")
    }
}

impl error::Error for InvalidScalarError {
    fn description(&self) -> &str {
        "Generated scalar was invalid (0 or 1)."
    }

    fn cause(&self) -> Option<&dyn error::Error> {
        None
    }
}

#[derive(PartialEq, Eq, Debug, Clone, Copy)]
pub struct CommitmentPair {
    random_secret: RandomSecret,
    commitment: Commitment,
}

impl CommitmentPair {
    pub fn new(random_secret: &RandomSecret, commitment: &Commitment) -> Self {
        let cloned_secret = *random_secret;
        let cloned_commitment = *commitment;
        CommitmentPair {
            random_secret: cloned_secret,
            commitment: cloned_commitment,
        }
    }

    fn generate_internal<R: Rng + RngCore + CryptoRng>(
        rng: &mut R,
    ) -> Result<CommitmentPair, InvalidScalarError> {
        // Create random 32 bytes.
        let mut randomness: [u8; RandomSecret::SIZE] = [0u8; RandomSecret::SIZE];
        rng.fill(&mut randomness);

        // Decompress the 32 byte cryptographically secure random data to 64 byte.
        let mut h: sha2::Sha512 = sha2::Sha512::default();

        h.update(randomness);
        let scalar = Scalar::from_hash::<sha2::Sha512>(h);
        if scalar == Scalar::ZERO || scalar == Scalar::ONE {
            return Err(InvalidScalarError);
        }

        // Compute the point [scalar]B.
        let commitment: EdwardsPoint = &scalar * constants::ED25519_BASEPOINT_TABLE;

        let rs = RandomSecret(scalar);
        let ct = Commitment(commitment);
        Ok(CommitmentPair {
            random_secret: rs,
            commitment: ct,
        })
    }

    #[inline]
    pub fn random_secret(&self) -> &RandomSecret {
        &self.random_secret
    }
    #[inline]
    pub fn commitment(&self) -> &Commitment {
        &self.commitment
    }
}

impl SecureGenerate for CommitmentPair {
    fn generate<R: Rng + RngCore + CryptoRng>(rng: &mut R) -> Self {
        CommitmentPair::generate_internal(rng).expect("Failed to generate CommitmentPair")
    }
}

#[derive(PartialEq, Eq, Debug, Clone, Copy)]
pub struct PartialSignature(pub Scalar);
implement_simple_add_sum_traits!(PartialSignature, Scalar::ZERO);

impl PartialSignature {
    pub const SIZE: usize = 32;

    pub fn to_signature(&self, aggregated_commitment: &Commitment) -> Signature {
        let mut signature: [u8; Signature::SIZE] = [0u8; Signature::SIZE];
        signature[..Commitment::SIZE].copy_from_slice(&aggregated_commitment.to_bytes());
        signature[Commitment::SIZE..].copy_from_slice(self.as_bytes());
        Signature::from(&signature)
    }

    #[inline]
    pub fn as_bytes(&self) -> &[u8; PartialSignature::SIZE] {
        self.0.as_bytes()
    }
}

impl From<[u8; PartialSignature::SIZE]> for PartialSignature {
    fn from(bytes: [u8; PartialSignature::SIZE]) -> Self {
        PartialSignature(Scalar::from_bytes_mod_order(bytes))
    }
}

impl<'a> From<&'a [u8; PartialSignature::SIZE]> for PartialSignature {
    fn from(bytes: &'a [u8; PartialSignature::SIZE]) -> Self {
        PartialSignature::from(*bytes)
    }
}

impl KeyPair {
    pub fn partial_sign(
        &self,
        public_keys: &[PublicKey],
        secret: &RandomSecret,
        commitments: &[Commitment],
        data: &[u8],
    ) -> (PartialSignature, PublicKey, Commitment) {
        assert!(
            public_keys.len() == commitments.len(),
            "Number of public keys and commitments must be the same."
        );
        assert!(
            !public_keys.is_empty(),
            "Number of public keys and commitments must be greater than 0."
        );
        assert!(
            public_keys.contains(&self.public),
            "Public keys must contain own key."
        );

        // Sort public keys.
        // public_keys.sort();

        // Hash public keys.
        let public_keys_hash = hash_public_keys(public_keys);
        // And delinearize them.
        let delinearized_pk_sum: EdwardsPoint = public_keys
            .iter()
            .map(|public_key| public_key.delinearize(&public_keys_hash))
            .sum();
        let delinearized_private_key: Scalar = self.delinearize_private_key(&public_keys_hash);

        // Aggregate commitments.
        let aggregated_commitment: Commitment = commitments.iter().sum();

        // Compute H(commitment || public key || message).
        let mut h: sha2::Sha512 = sha2::Sha512::default();

        h.update(aggregated_commitment.0.compress().as_bytes());
        h.update(delinearized_pk_sum.compress().as_bytes());
        h.update(data);
        let s = Scalar::from_hash::<sha2::Sha512>(h);
        let partial_signature: Scalar = s * delinearized_private_key + secret.0;
        let mut public_key_bytes: [u8; PublicKey::SIZE] = [0u8; PublicKey::SIZE];
        public_key_bytes.copy_from_slice(delinearized_pk_sum.compress().as_bytes());
        (
            PartialSignature(partial_signature),
            PublicKey::from(public_key_bytes),
            aggregated_commitment,
        )
    }

    pub fn delinearize_private_key(&self, public_keys_hash: &[u8; 64]) -> Scalar {
        // Compute H(C||P).
        let mut h: sha2::Sha512 = sha2::Sha512::default();

        h.update(&public_keys_hash[..]);
        h.update(self.public.as_bytes());
        let s = Scalar::from_hash::<sha2::Sha512>(h);

        // Get a scalar representation of the private key
        let sk = self.private.0.to_scalar();

        // Compute H(C||P)*sk
        s * sk
    }
}

impl PublicKey {
    fn to_edwards_point(self) -> Option<EdwardsPoint> {
        let mut bits: [u8; PublicKey::SIZE] = [0u8; PublicKey::SIZE];
        bits.copy_from_slice(&self.as_bytes()[..PublicKey::SIZE]);

        let compressed = CompressedEdwardsY(bits);
        compressed.decompress()
    }

    pub fn delinearize(&self, public_keys_hash: &[u8; 64]) -> EdwardsPoint {
        // Compute H(C||P).
        let mut h: sha2::Sha512 = sha2::Sha512::default();

        h.update(&public_keys_hash[..]);
        h.update(self.as_bytes());
        let s = Scalar::from_hash::<sha2::Sha512>(h);

        // Should always work, since we come from a valid public key.
        let p = self.to_edwards_point().unwrap();
        // Compute H(C||P)*P.
        s * p
    }
}

pub fn hash_public_keys(public_keys: &[PublicKey]) -> [u8; 64] {
    // 1. Compute hash over public keys public_keys_hash = C = H(P_1 || ... || P_n).
    let mut h: sha2::Sha512 = sha2::Sha512::default();
    let mut public_keys_hash: [u8; 64] = [0u8; 64];
    for public_key in public_keys {
        h.update(public_key.as_bytes());
    }
    public_keys_hash.copy_from_slice(h.finalize().as_slice());
    public_keys_hash
}

trait ToScalar {
    fn to_scalar(&self) -> Scalar;
}

impl ToScalar for ::ed25519_zebra::SigningKey {
    fn to_scalar(&self) -> Scalar {
        // Expand the seed to a 64-byte array with SHA512.
        let h = Sha512::digest(self.as_ref());

        // Convert the low half to a scalar with Ed25519 "clamping"
        let mut scalar_bytes = [0u8; 32];
        scalar_bytes[..].copy_from_slice(&h.as_slice()[0..32]);
        scalar_bytes[0] &= 248;
        scalar_bytes[31] &= 127;
        scalar_bytes[31] |= 64;
        // The above bit operations ensure that the integer represented by
        // `scalar_bytes` is less than 2***255-19 as required by this function.
        #[allow(deprecated)]
        Scalar::from_bits(scalar_bytes)
    }
}
