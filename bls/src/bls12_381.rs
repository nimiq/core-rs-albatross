use byteorder::{BigEndian, ByteOrder};
use pairing::{CurveAffine, CurveProjective, EncodedPoint, GroupDecodingError, Engine};
use pairing::{PrimeField, PrimeFieldDecodingError};
use pairing::bls12_381::{Bls12, Fr, FrRepr, G1Compressed, G2Compressed};

use crate::Encoding;

use super::{
    AggregatePublicKey as GenericAggregatePublicKey,
    AggregateSignature as GenericAggregateSignature,
    Keypair as GenericKeyPair,
    PublicKey as GenericPublicKey,
    SecretKey as GenericSecretKey,
    Signature as GenericSignature,
};

pub type PublicKey = GenericPublicKey<Bls12>;
pub type SecretKey = GenericSecretKey<Bls12>;
pub type Signature = GenericSignature<Bls12>;
pub type KeyPair = GenericKeyPair<Bls12>;

pub type AggregatePublicKey = GenericAggregatePublicKey<Bls12>;
pub type AggregateSignature = GenericAggregateSignature<Bls12>;

pub type Hash = <Bls12 as Engine>::G1Affine;

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
        assert_eq!(repr.as_ref().len(), Self::SIZE);
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
