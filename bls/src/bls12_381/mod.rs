use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash as StdHash, Hasher};
use std::str::FromStr;

use failure::Fail;
use group::{CurveAffine, CurveProjective, EncodedPoint, GroupDecodingError};
use hex::FromHexError;
use pairing::bls12_381::{Bls12, G1Compressed, G2Compressed};
use pairing::Engine;

#[cfg(feature = "beserial")]
use beserial::{Deserialize, Serialize};
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
        self.as_affine().partial_cmp(&other.as_affine())
    }
}

impl Ord for PublicKey {
    fn cmp(&self, other: &Self) -> Ordering {
        self.as_affine().cmp(&other.as_affine())
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

    pub fn verify<M: Hash>(&self, msg: &M, signature: &Signature) -> bool {
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

    pub fn as_projective(&self) -> PublicKey {
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
        write!(f, "PublicKey({})", self.compress().to_hex())
    }
}

impl fmt::Display for PublicKey {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(f, "{}", self.compress().to_hex())
    }
}

impl PublicKey {
    pub fn compress(&self) -> CompressedPublicKey {
        CompressedPublicKey {
            p_pub: self.p_pub.into_affine().into_compressed(),
        }
    }

    pub fn as_affine(&self) -> PublicKeyAffine {
        PublicKeyAffine {
            p_pub: self.p_pub.into_affine(),
        }
    }
}

#[derive(Clone)]
pub struct CompressedPublicKey {
    pub(crate) p_pub: G2Compressed,
}

impl fmt::Debug for CompressedPublicKey {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(f, "CompressedPublicKey({})", &self.to_hex())
    }
}

impl fmt::Display for CompressedPublicKey {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(f, "{}", &self.to_hex())
    }
}

impl PartialEq for CompressedPublicKey {
    fn eq(&self, other: &CompressedPublicKey) -> bool {
        self.p_pub.as_ref() == other.p_pub.as_ref()
    }
}

impl StdHash for CompressedPublicKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        StdHash::hash(self.p_pub.as_ref(), state)
    }
}

impl Eq for CompressedPublicKey {}

impl PartialOrd<CompressedPublicKey> for CompressedPublicKey {
    fn partial_cmp(&self, other: &CompressedPublicKey) -> Option<Ordering> {
        self.p_pub.as_ref().partial_cmp(other.p_pub.as_ref())
    }
}

impl Ord for CompressedPublicKey {
    fn cmp(&self, other: &Self) -> Ordering {
        self.p_pub.as_ref().cmp(other.p_pub.as_ref())
    }
}

impl AsRef<[u8]> for CompressedPublicKey {
    fn as_ref(&self) -> &[u8] {
        self.p_pub.as_ref()
    }
}

#[derive(Clone, Debug, Fail)]
pub enum PublicKeyParseError {
    #[fail(display = "Not valid hexadecimal: {}", _0)]
    InvalidHex(#[cause] FromHexError),
    #[fail(display = "Incorrect length: {}", _0)]
    IncorrectLength(usize),
}

impl From<FromHexError> for PublicKeyParseError {
    fn from(e: FromHexError) -> Self {
        PublicKeyParseError::InvalidHex(e)
    }
}

#[cfg(feature = "beserial")]
impl FromStr for CompressedPublicKey {
    type Err = PublicKeyParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let raw = hex::decode(s)?;
        if raw.len() != CompressedPublicKey::SIZE {
            return Err(PublicKeyParseError::IncorrectLength(raw.len()));
        }
        Ok(CompressedPublicKey::deserialize_from_vec(&raw).unwrap())
    }
}

impl CompressedPublicKey {
    pub const SIZE: usize = 96;

    pub fn uncompress(&self) -> Result<PublicKey, GroupDecodingError> {
        Ok(PublicKey {
            p_pub: self.p_pub.into_affine()?.into_projective()
        })
    }

    pub fn to_hex(&self) -> String {
        hex::encode(self.p_pub.as_ref())
    }
}

impl fmt::Display for Signature {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(f, "{}", self.compress().to_hex())
    }
}

impl fmt::Debug for Signature {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(f, "Signature({})", &::hex::encode(self.compress().as_ref()))
    }
}

impl Signature {
    pub fn compress(&self) -> CompressedSignature {
        CompressedSignature {
            s: self.s.into_affine().into_compressed(),
        }
    }
}

#[derive(Clone, Copy)]
pub struct CompressedSignature {
    pub(crate) s: G1Compressed,
}

impl fmt::Debug for CompressedSignature {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(f, "CompressedSignature({})", self.to_hex())
    }
}

impl fmt::Display for CompressedSignature {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(f, "{}", self.to_hex())
    }
}

impl PartialEq for CompressedSignature {
    fn eq(&self, other: &CompressedSignature) -> bool {
        self.s.as_ref() == other.s.as_ref()
    }
}

impl Eq for CompressedSignature {}

impl PartialOrd<CompressedSignature> for CompressedSignature {
    fn partial_cmp(&self, other: &CompressedSignature) -> Option<Ordering> {
        self.s.as_ref().partial_cmp(other.s.as_ref())
    }
}

impl Ord for CompressedSignature {
    fn cmp(&self, other: &Self) -> Ordering {
        self.s.as_ref().cmp(other.s.as_ref())
    }
}

impl AsRef<[u8]> for CompressedSignature {
    fn as_ref(&self) -> &[u8] {
        self.s.as_ref()
    }
}

impl AsMut<[u8]> for CompressedSignature {
    fn as_mut(&mut self) -> &mut [u8] {
        self.s.as_mut()
    }
}

impl Default for CompressedSignature {
    fn default() -> Self {
        CompressedSignature {
            s: G1Compressed::empty(),
        }
    }
}

impl CompressedSignature {
    pub const SIZE: usize = 48;

    pub fn uncompress(&self) -> Result<Signature, GroupDecodingError> {
        Ok(Signature {
            s: self.s.into_affine()?.into_projective()
        })
    }

    pub fn to_hex(&self) -> String {
        hex::encode(self.s.as_ref())
    }
}

impl fmt::Display for AggregateSignature {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        fmt::Display::fmt(&self.0, f)
    }
}

impl fmt::Debug for AggregateSignature {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        fmt::Debug::fmt(&self.0, f)
    }
}

impl fmt::Display for AggregatePublicKey {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        fmt::Display::fmt(&self.0, f)
    }
}

impl fmt::Debug for AggregatePublicKey {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        fmt::Debug::fmt(&self.0, f)
    }
}
