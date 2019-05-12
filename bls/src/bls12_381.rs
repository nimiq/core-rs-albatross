use std::cmp::Ordering;

use byteorder::{BigEndian, ByteOrder};
use ff::{PrimeField, PrimeFieldDecodingError};
use group::{CurveAffine, CurveProjective, EncodedPoint, GroupDecodingError};
use pairing::bls12_381::{Bls12, Fr, FrRepr, G1Compressed, G2Compressed};
use pairing::Engine;

use hash::Hash;

use super::{
    AggregatePublicKey as GenericAggregatePublicKey,
    AggregateSignature as GenericAggregateSignature,
    hash_to_g1,
    Keypair as GenericKeyPair,
    PublicKey as GenericPublicKey,
    SecretKey as GenericSecretKey,
    Signature as GenericSignature,
    Encoding,
    SigHash
};

pub type PublicKey = GenericPublicKey<Bls12>;
pub type SecretKey = GenericSecretKey<Bls12>;
pub type Signature = GenericSignature<Bls12>;
pub type KeyPair = GenericKeyPair<Bls12>;

impl PartialOrd<PublicKey> for PublicKey {
    fn partial_cmp(&self, other: &PublicKey) -> Option<Ordering> {
        PublicKeyAffine::from(self.clone()).partial_cmp(&PublicKeyAffine::from(other.clone()))
    }
}

impl Ord for PublicKey {
    fn cmp(&self, other: &Self) -> Ordering {
        PublicKeyAffine::from(self.clone()).cmp(&PublicKeyAffine::from(other.clone()))
    }
}

pub type AggregatePublicKey = GenericAggregatePublicKey<Bls12>;
pub type AggregateSignature = GenericAggregateSignature<Bls12>;

#[derive(Clone, Copy, Debug, PartialEq, Eq,)]
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

impl Encoding for PublicKey {
    type Error = GroupDecodingError;
    type ByteArray = [u8; 96];
    const SIZE: usize = 96;

    fn to_bytes(&self) -> Self::ByteArray {
        let mut key = [0; Self::SIZE];
        let repr = self.p_pub.into_affine().into_compressed();
        assert_eq!(G2Compressed::size(), Self::SIZE);
        key.copy_from_slice(repr.as_ref());
        key
    }

    fn from_bytes(bytes: Self::ByteArray) -> Result<Self, Self::Error> {
        Self::from_slice(bytes.as_ref())
    }

    fn from_slice(bytes: &[u8]) -> Result<Self, Self::Error> {
        if bytes.len() != Self::SIZE {
            return Err(GroupDecodingError::UnexpectedInformation);
        }

        let mut point = G2Compressed::empty();
        point.as_mut().copy_from_slice(&bytes);
        Ok(PublicKey {
            p_pub: point.into_affine()?.into_projective(),
        })
    }
}

impl Encoding for SecretKey {
    type Error = PrimeFieldDecodingError;
    type ByteArray = [u8; 32];
    const SIZE: usize = 32;

    fn to_bytes(&self) -> Self::ByteArray {
        let mut key = [0u8; Self::SIZE];
        let repr = self.x.into_repr();
        // repr.as_ref() returns aslice of u64's, so multiply with 8!
        assert_eq!(repr.as_ref().len() * 8, Self::SIZE);
        BigEndian::write_u64_into(repr.as_ref(), key.as_mut());
        key
    }

    fn from_bytes(bytes: Self::ByteArray) -> Result<Self, Self::Error> {
        Self::from_slice(bytes.as_ref())
    }

    fn from_slice(bytes: &[u8]) -> Result<Self, Self::Error> {
        if bytes.len() != Self::SIZE {
            #[cfg(feature = "std")]
                return Err(PrimeFieldDecodingError::NotInField("Invalid size".to_string()));
            #[cfg(not(feature = "std"))]
            panic!("Invalid slice size!");
        }

        let mut element = FrRepr::default();
        BigEndian::read_u64_into(bytes, element.as_mut());
        Ok(SecretKey {
            x: Fr::from_repr(element)?,
        })
    }
}

impl Encoding for Signature {
    type Error = GroupDecodingError;
    type ByteArray = [u8; 48];
    const SIZE: usize = 48;

    fn to_bytes(&self) -> Self::ByteArray {
        let mut key = [0; Self::SIZE];
        let repr = self.s.into_affine().into_compressed();
        assert_eq!(G1Compressed::size(), Self::SIZE);
        key.copy_from_slice(repr.as_ref());
        key
    }

    fn from_bytes(bytes: Self::ByteArray) -> Result<Self, Self::Error> {
        Self::from_slice(bytes.as_ref())
    }

    fn from_slice(bytes: &[u8]) -> Result<Self, Self::Error> {
        if bytes.len() != Self::SIZE {
            return Err(GroupDecodingError::UnexpectedInformation);
        }

        let mut point = G1Compressed::empty();
        point.as_mut().copy_from_slice(&bytes);
        Ok(Signature {
            s: point.into_affine()?.into_projective(),
        })
    }
}

impl Encoding for AggregateSignature {
    type Error = GroupDecodingError;
    type ByteArray = [u8; 48];
    const SIZE: usize = 48;

    fn to_bytes(&self) -> Self::ByteArray {
        let mut key = [0; Self::SIZE];
        let repr = self.0.s.into_affine().into_compressed();
        assert_eq!(G1Compressed::size(), Self::SIZE);
        key.copy_from_slice(repr.as_ref());
        key
    }

    fn from_bytes(bytes: Self::ByteArray) -> Result<Self, Self::Error> {
        Self::from_slice(bytes.as_ref())
    }

    fn from_slice(bytes: &[u8]) -> Result<Self, Self::Error> {
        if bytes.len() != Self::SIZE {
            return Err(GroupDecodingError::UnexpectedInformation);
        }

        let mut point = G1Compressed::empty();
        point.as_mut().copy_from_slice(&bytes);
        Ok(GenericAggregateSignature(Signature {
            s: point.into_affine()?.into_projective(),
        }))
    }
}

impl Encoding for AggregatePublicKey {
    type Error = GroupDecodingError;
    type ByteArray = [u8; 96];
    const SIZE: usize = 96;

    fn to_bytes(&self) -> Self::ByteArray {
        let mut key = [0; Self::SIZE];
        let repr = self.0.p_pub.into_affine().into_compressed();
        assert_eq!(G2Compressed::size(), Self::SIZE);
        key.copy_from_slice(repr.as_ref());
        key
    }

    fn from_bytes(bytes: Self::ByteArray) -> Result<Self, Self::Error> {
        Self::from_slice(bytes.as_ref())
    }

    fn from_slice(bytes: &[u8]) -> Result<Self, Self::Error> {
        if bytes.len() != Self::SIZE {
            return Err(GroupDecodingError::UnexpectedInformation);
        }

        let mut point = G2Compressed::empty();
        point.as_mut().copy_from_slice(&bytes);
        Ok(GenericAggregatePublicKey(PublicKey {
            p_pub: point.into_affine()?.into_projective(),
        }))
    }
}
