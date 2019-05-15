use std::cmp::Ordering;
use std::fmt;

use group::{CurveAffine, CurveProjective, EncodedPoint, GroupDecodingError};
use pairing::bls12_381::{Bls12, G1Compressed, G2Compressed};
use pairing::Engine;

#[cfg(feature = "beserial")]
use beserial::Serialize;
use hash::Hash;

use super::{
    AggregatePublicKey as GenericAggregatePublicKey,
    AggregateSignature as GenericAggregateSignature,
    hash_to_g1,
    KeyPair as GenericKeyPair,
    PublicKey as GenericPublicKey,
    SecretKey as GenericSecretKey,
    SigHash,
    Signature as GenericSignature
};

#[cfg(feature = "lazy")]
pub mod lazy;

pub type PublicKey = GenericPublicKey<Bls12>;
pub type SecretKey = GenericSecretKey<Bls12>;
pub type Signature = GenericSignature<Bls12>;
pub type KeyPair = GenericKeyPair<Bls12>;

impl SecretKey {
    pub const SIZE: usize = 32;
}

#[cfg(feature = "beserial")]
impl fmt::Debug for KeyPair {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        f.write_str(&::hex::encode(self.serialize_to_vec()))
    }
}

#[cfg(feature = "beserial")]
impl fmt::Debug for SecretKey {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        f.write_str(&::hex::encode(self.serialize_to_vec()))
    }
}

impl PartialOrd<PublicKey> for PublicKey {
    fn partial_cmp(&self, other: &PublicKey) -> Option<Ordering> {
        self.into_affine().partial_cmp(&other.into_affine())
    }
}

impl Ord for PublicKey {
    fn cmp(&self, other: &Self) -> Ordering {
        self.into_affine().cmp(&other.into_affine())
    }
}

pub type AggregatePublicKey = GenericAggregatePublicKey<Bls12>;
pub type AggregateSignature = GenericAggregateSignature<Bls12>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PublicKeyAffine {
    pub(crate) p_pub: <Bls12 as Engine>::G2Affine,
}

impl PartialOrd<PublicKeyAffine> for PublicKeyAffine {
    fn partial_cmp(&self, other: &PublicKeyAffine) -> Option<Ordering> {
        Some(self.p_pub.lexicographic_cmp(&other.p_pub))
    }
}

impl Ord for PublicKeyAffine {
    fn cmp(&self, other: &Self) -> Ordering {
        self.p_pub.lexicographic_cmp(&other.p_pub)
    }
}

impl PublicKeyAffine {
    pub fn from_secret(secret: &SecretKey) -> Self {
        PublicKey::from_secret(secret).into()
    }

    pub fn verify<M: Hash>(&self, msg: M, signature: &Signature) -> bool {
        self.verify_g1(hash_to_g1::<Bls12>(msg.hash()), signature)
    }

    pub fn verify_hash(&self, hash: SigHash, signature: &Signature) -> bool {
        self.verify_g1(hash_to_g1::<Bls12>(hash), signature)
    }

    fn verify_g1<H: Into<<Bls12 as Engine>::G1Affine>>(&self, hash: H, signature: &Signature) -> bool {
        let lhs = Bls12::pairing(signature.s, <Bls12 as Engine>::G2Affine::one());
        let rhs = Bls12::pairing(hash.into(), self.p_pub.into_projective());
        lhs == rhs
    }

    pub fn into_projective(&self) -> PublicKey {
        PublicKey {
            p_pub: self.p_pub.into_projective(),
        }
    }
}

impl From<PublicKey> for PublicKeyAffine {
    fn from(public_key: PublicKey) -> Self {
        PublicKeyAffine {
            p_pub: public_key.p_pub.into_affine(),
        }
    }
}

impl From<PublicKeyAffine> for PublicKey {
    fn from(public_key: PublicKeyAffine) -> Self {
        PublicKey {
            p_pub: public_key.p_pub.into_projective(),
        }
    }
}

impl fmt::Debug for PublicKey {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        f.write_str(&::hex::encode(self.compress().as_ref()))
    }
}

impl PublicKey {
    pub fn compress(&self) -> CompressedPublicKey {
        CompressedPublicKey {
            p_pub: self.p_pub.into_affine().into_compressed(),
        }
    }

    pub fn into_affine(&self) -> PublicKeyAffine {
        PublicKeyAffine {
            p_pub: self.p_pub.into_affine(),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct CompressedPublicKey {
    pub(crate) p_pub: G2Compressed,
}

impl PartialEq for CompressedPublicKey {
    fn eq(&self, other: &CompressedPublicKey) -> bool {
        self.p_pub.as_ref() == other.p_pub.as_ref()
    }
}

impl Eq for CompressedPublicKey {}

impl PartialOrd<CompressedPublicKey> for CompressedPublicKey {
    fn partial_cmp(&self, other: &CompressedPublicKey) -> Option<Ordering> {
        self.p_pub.as_ref().partial_cmp(&other.p_pub.as_ref())
    }
}

impl Ord for CompressedPublicKey {
    fn cmp(&self, other: &Self) -> Ordering {
        self.p_pub.as_ref().cmp(&other.p_pub.as_ref())
    }
}

impl AsRef<[u8]> for CompressedPublicKey {
    fn as_ref(&self) -> &[u8] {
        self.p_pub.as_ref()
    }
}

impl CompressedPublicKey {
    pub const SIZE: usize = 96;

    pub fn uncompress(&self) -> Result<PublicKey, GroupDecodingError> {
        Ok(PublicKey {
            p_pub: self.p_pub.into_affine()?.into_projective()
        })
    }
}

impl fmt::Debug for Signature {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        f.write_str(&::hex::encode(self.compress().as_ref()))
    }
}

impl Signature {
    pub fn compress(&self) -> CompressedSignature {
        CompressedSignature {
            s: self.s.into_affine().into_compressed(),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct CompressedSignature {
    pub(crate) s: G1Compressed,
}

impl PartialEq for CompressedSignature {
    fn eq(&self, other: &CompressedSignature) -> bool {
        self.s.as_ref() == other.s.as_ref()
    }
}

impl Eq for CompressedSignature {}

impl PartialOrd<CompressedSignature> for CompressedSignature {
    fn partial_cmp(&self, other: &CompressedSignature) -> Option<Ordering> {
        self.s.as_ref().partial_cmp(&other.s.as_ref())
    }
}

impl Ord for CompressedSignature {
    fn cmp(&self, other: &Self) -> Ordering {
        self.s.as_ref().cmp(&other.s.as_ref())
    }
}

impl AsRef<[u8]> for CompressedSignature {
    fn as_ref(&self) -> &[u8] {
        self.s.as_ref()
    }
}

impl CompressedSignature {
    pub const SIZE: usize = 48;

    pub fn uncompress(&self) -> Result<Signature, GroupDecodingError> {
        Ok(Signature {
            s: self.s.into_affine()?.into_projective()
        })
    }
}

impl fmt::Debug for AggregateSignature {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        self.0.fmt(f)
    }
}

impl fmt::Debug for AggregatePublicKey {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        self.0.fmt(f)
    }
}
