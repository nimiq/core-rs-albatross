use std::io::Error;

use algebra::{BigInteger384, Zero, PrimeField};
use algebra::curves::models::SWModelParameters as Parameters;
use algebra::fields::bls12_377::{Fq, Fq2};
use byteorder::{ReadBytesExt, WriteBytesExt, BigEndian};
use algebra::curves::models::short_weierstrass_jacobian::GroupAffine;

/// Serializer in big endian format.
pub trait BeSerialize {
    /// Serializes `self` into `writer`.
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<(), Error> {
        BeSerialize::serialize_with_flags(self, writer, Default::default())
    }
    /// Serializes `self` and `flags` into `writer`.
    fn serialize_with_flags<W: WriteBytesExt>(&self, writer: &mut W, _flags: Flags) -> Result<(), Error>;
    fn serialized_size(&self) -> usize;
}

/// Deserializer in big endian format.
pub trait BeDeserialize: Sized {
    /// Reads `Self` from `reader`.
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, Error>;
    /// Reads `Self` and `Flags` from `reader`.
    /// Returns empty flags by default.
    fn deserialize_with_flags<R: ReadBytesExt>(reader: &mut R) -> Result<(Self, Flags), Error>;
}

/// Flags to be encoded into the serialization.
/// The default flags (empty) should not change the binary representation.
#[derive(Default, Clone, Copy)]
pub struct Flags {
    pub y_sign: bool,
    pub is_infinity: bool,
}

impl Flags {
    pub fn infinity() -> Self {
        Flags {
            y_sign: false,
            is_infinity: true,
        }
    }

    pub fn y_sign(sign: bool) -> Self {
        Flags {
            y_sign: sign,
            is_infinity: false,
        }
    }

    pub fn u64_bitmask(&self) -> u64 {
        let mut mask = 0;
        if self.y_sign {
            mask |= 1 << 63;
        }
        if self.is_infinity {
            mask |= 1 << 62;
        }
        mask
    }

    pub fn from_u64(value: u64) -> Self {
        let x_sign = (value >> 63) & 1 == 1;
        let is_infinity = (value >> 62) & 1 == 1;
        Flags {
            y_sign: x_sign,
            is_infinity
        }
    }

    pub fn from_u64_remove_flags(value: &mut u64) -> Self {
        let flags = Self::from_u64(*value);
        *value &= 0x3FFF_FFFF_FFFF_FFFF;
        flags
    }
}

// GroupAffine
impl<P: Parameters> BeSerialize for GroupAffine<P>
    where P::BaseField: BeSerialize {
    fn serialize_with_flags<W: WriteBytesExt>(&self, writer: &mut W, _flags: Flags) -> Result<(), Error> {
        // We always ignore flags here.
        if self.is_zero() {
            let flags = Flags::infinity();
            // Serialize 0.
            BeSerialize::serialize_with_flags(&P::BaseField::zero(), writer, flags)
        } else {
            let flags = Flags::y_sign(self.y > -self.y);
            BeSerialize::serialize_with_flags(&self.x, writer, flags)
        }
    }

    fn serialized_size(&self) -> usize {
        BeSerialize::serialized_size(&self.x)
    }
}

impl<P: Parameters> BeDeserialize for GroupAffine<P>
    where P::BaseField: BeDeserialize {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, Error> {
        let (point, _): (Self, _) = BeDeserialize::deserialize_with_flags(reader)?;
        Ok(point)
    }

    fn deserialize_with_flags<R: ReadBytesExt>(reader: &mut R) -> Result<(Self, Flags), Error> {
        let (x, flags): (P::BaseField, Flags) = BeDeserialize::deserialize_with_flags(reader)?;
        if flags.is_infinity {
            Ok((Self::zero(), flags))
        } else {
            Ok(
                (GroupAffine::get_point_from_x(x, flags.y_sign)
                     .ok_or(Error::from(std::io::ErrorKind::InvalidData))?,
                 flags)
            )
        }
    }
}

// Fq
impl BeSerialize for Fq {
    fn serialize_with_flags<W: WriteBytesExt>(&self, writer: &mut W, flags: Flags) -> Result<(), Error> {
        BeSerialize::serialize_with_flags(&self.into_repr(), writer, flags)
    }

    fn serialized_size(&self) -> usize {
        BeSerialize::serialized_size(&self.into_repr())
    }
}

impl BeDeserialize for Fq {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, Error> {
        let value: BigInteger384 = BeDeserialize::deserialize(reader)?;
        Ok(Fq::from_repr(value))
    }

    fn deserialize_with_flags<R: ReadBytesExt>(reader: &mut R) -> Result<(Self, Flags), Error> {
        let (value, flags): (BigInteger384, _) = BeDeserialize::deserialize_with_flags(reader)?;
        Ok((Fq::from_repr(value), flags))
    }
}

// Fq2
impl BeSerialize for Fq2 {
    fn serialize_with_flags<W: WriteBytesExt>(&self, writer: &mut W, flags: Flags) -> Result<(), Error> {
        BeSerialize::serialize_with_flags(&self.c0, writer, flags)?;
        BeSerialize::serialize(&self.c1, writer)
    }

    fn serialized_size(&self) -> usize {
        BeSerialize::serialized_size(&self.c0) + BeSerialize::serialized_size(&self.c1)
    }
}

impl BeDeserialize for Fq2 {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, Error> {
        let c0: Fq = BeDeserialize::deserialize(reader)?;
        let c1: Fq = BeDeserialize::deserialize(reader)?;
        Ok(Fq2::new(c0, c1))
    }

    fn deserialize_with_flags<R: ReadBytesExt>(reader: &mut R) -> Result<(Self, Flags), Error> {
        let (c0, flags): (Fq, Flags) = BeDeserialize::deserialize_with_flags(reader)?;
        let c1: Fq = BeDeserialize::deserialize(reader)?;
        Ok((Fq2::new(c0, c1), flags))
    }
}

// BigInteger384
impl BeSerialize for BigInteger384 {
    fn serialize_with_flags<W: WriteBytesExt>(&self, writer: &mut W, flags: Flags) -> Result<(), Error> {
        // BigInteger stores the least significant u64 first, so we need to reverse the order.
        for (i, &limb) in self.as_ref().iter().rev().enumerate() {
            let mut limb_to_encode = limb;
            // Encode flags into very first u64 at most significant bits.
            if i == 0 {
                limb_to_encode |= flags.u64_bitmask();
            }
            writer.write_u64::<BigEndian>(limb_to_encode)?;
        }
        Ok(())
    }

    fn serialized_size(&self) -> usize {
        48
    }
}

impl BeDeserialize for BigInteger384 {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, Error> {
        let mut limbs = [0u64; 6];
        for limb in limbs.iter_mut().rev() {
            *limb = reader.read_u64::<BigEndian>()?;
        }
        Ok(BigInteger384::new(limbs))
    }

    fn deserialize_with_flags<R: ReadBytesExt>(reader: &mut R) -> Result<(Self, Flags), Error> {
        let mut limbs = [0u64; 6];
        let mut flags = Default::default();
        for (i, limb) in limbs.iter_mut().rev().enumerate() {
            *limb = reader.read_u64::<BigEndian>()?;
            // The first limb encodes the flags, so remove them.
            if i == 0 {
                flags = Flags::from_u64_remove_flags(limb);
            }
        }
        Ok((BigInteger384::new(limbs), flags))
    }
}
