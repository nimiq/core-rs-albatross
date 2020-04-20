use std::io::{Error, ErrorKind};

use algebra::mnt4_753::{Fq as MNT4Fq, Fq2 as MNT4Fq2};
use algebra::mnt6_753::{Fq as MNT6Fq, Fq3 as MNT6Fq3};
use algebra::short_weierstrass_jacobian::GroupAffine;
use algebra::BigInteger768;
use algebra_core::curves::models::SWModelParameters;
use algebra_core::fields::PrimeField;
use algebra_core::Zero;
use byteorder::{BigEndian, ByteOrder, ReadBytesExt, WriteBytesExt};

/// Serializer in big endian format.
pub trait BeSerialize {
    /// Serializes `self` into `writer`.
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<(), Error> {
        BeSerialize::serialize_with_flags(self, writer, Default::default())
    }
    /// Serializes `self` and `flags` into `writer`.
    fn serialize_with_flags<W: WriteBytesExt>(
        &self,
        writer: &mut W,
        _flags: Flags,
    ) -> Result<(), Error>;
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

/// (De)serializer that allows to set a custom length.
/// This is used for the BigInteger types that always are a multiple of 64 bits.
trait CustomLengthDeSerialize: Sized {
    /// Serializes `self` and `flags` into `writer` with a custom length `length`.
    fn serialize_with_flags<W: WriteBytesExt>(
        &self,
        writer: &mut W,
        flags: Flags,
        length: usize,
    ) -> Result<(), Error>;

    /// Reads `Self` from `reader`, reading `length` bytes and padding with 0s.
    fn deserialize<R: ReadBytesExt>(reader: &mut R, length: usize) -> Result<Self, Error>;

    /// Reads `Self` and `Flags` from `reader`, reading `length` bytes and padding with 0s.
    /// Returns empty flags by default.
    fn deserialize_with_flags<R: ReadBytesExt>(
        reader: &mut R,
        length: usize,
    ) -> Result<(Self, Flags), Error>;
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

    /// Returns a u64 bitmask with the two flags set encoded into the most significant bits
    /// (given an offset of 0).
    /// The `offset` can be used to move the flags by `offset` bits.
    pub fn u64_bitmask(&self, offset: u64) -> u64 {
        let mut mask = 0;
        if self.y_sign {
            mask |= 1 << (63 - offset);
        }
        if self.is_infinity {
            mask |= 1 << (62 - offset);
        }
        mask
    }

    /// Returns the flags encoded into a u64 into the most significant bits (minus a bit offset).
    pub fn from_u64(value: u64, offset: u64) -> Self {
        let x_sign = (value >> (63 - offset)) & 1 == 1;
        let is_infinity = (value >> (62 - offset)) & 1 == 1;
        Flags {
            y_sign: x_sign,
            is_infinity,
        }
    }

    /// Removes and returns the flags encoded into a u64 into the most significant bits (minus a bit offset).
    pub fn from_u64_remove_flags(value: &mut u64, offset: u64) -> Self {
        let flags = Self::from_u64(*value, offset);
        *value &= !(3 << (62 - offset));
        flags
    }
}

// GroupAffine
impl<P: SWModelParameters> BeSerialize for GroupAffine<P>
where
    P::BaseField: BeSerialize,
{
    fn serialize_with_flags<W: WriteBytesExt>(
        &self,
        writer: &mut W,
        _flags: Flags,
    ) -> Result<(), Error> {
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

impl<P: SWModelParameters> BeDeserialize for GroupAffine<P>
where
    P::BaseField: BeDeserialize,
{
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, Error> {
        let (point, _): (Self, _) = BeDeserialize::deserialize_with_flags(reader)?;
        Ok(point)
    }

    fn deserialize_with_flags<R: ReadBytesExt>(reader: &mut R) -> Result<(Self, Flags), Error> {
        let (x, flags): (P::BaseField, Flags) = BeDeserialize::deserialize_with_flags(reader)?;
        if flags.is_infinity {
            Ok((Self::zero(), flags))
        } else {
            Ok((
                GroupAffine::get_point_from_x(x, flags.y_sign)
                    .ok_or(Error::from(std::io::ErrorKind::InvalidData))?,
                flags,
            ))
        }
    }
}

// MNT6 Fq
impl BeSerialize for MNT6Fq {
    fn serialize_with_flags<W: WriteBytesExt>(
        &self,
        writer: &mut W,
        flags: Flags,
    ) -> Result<(), Error> {
        CustomLengthDeSerialize::serialize_with_flags(&self.into_repr(), writer, flags, 95)
    }

    fn serialized_size(&self) -> usize {
        95
    }
}

impl BeDeserialize for MNT6Fq {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, Error> {
        let value: BigInteger768 = CustomLengthDeSerialize::deserialize(reader, 95)?;
        Ok(MNT6Fq::from_repr(value))
    }

    fn deserialize_with_flags<R: ReadBytesExt>(reader: &mut R) -> Result<(Self, Flags), Error> {
        let (value, flags): (BigInteger768, _) =
            CustomLengthDeSerialize::deserialize_with_flags(reader, 95)?;
        Ok((MNT6Fq::from_repr(value), flags))
    }
}

// MNT6 Fq3
impl BeSerialize for MNT6Fq3 {
    fn serialize_with_flags<W: WriteBytesExt>(
        &self,
        writer: &mut W,
        flags: Flags,
    ) -> Result<(), Error> {
        BeSerialize::serialize_with_flags(&self.c0, writer, flags)?;
        BeSerialize::serialize(&self.c1, writer)?;
        BeSerialize::serialize(&self.c2, writer)
    }

    fn serialized_size(&self) -> usize {
        BeSerialize::serialized_size(&self.c0)
            + BeSerialize::serialized_size(&self.c1)
            + BeSerialize::serialized_size(&self.c2)
    }
}

impl BeDeserialize for MNT6Fq3 {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, Error> {
        let c0: MNT6Fq = BeDeserialize::deserialize(reader)?;
        let c1: MNT6Fq = BeDeserialize::deserialize(reader)?;
        let c2: MNT6Fq = BeDeserialize::deserialize(reader)?;
        Ok(MNT6Fq3::new(c0, c1, c2))
    }

    fn deserialize_with_flags<R: ReadBytesExt>(reader: &mut R) -> Result<(Self, Flags), Error> {
        let (c0, flags): (MNT6Fq, Flags) = BeDeserialize::deserialize_with_flags(reader)?;
        let c1: MNT6Fq = BeDeserialize::deserialize(reader)?;
        let c2: MNT6Fq = BeDeserialize::deserialize(reader)?;
        Ok((MNT6Fq3::new(c0, c1, c2), flags))
    }
}

// MNT4 Fq
impl BeSerialize for MNT4Fq {
    fn serialize_with_flags<W: WriteBytesExt>(
        &self,
        writer: &mut W,
        flags: Flags,
    ) -> Result<(), Error> {
        CustomLengthDeSerialize::serialize_with_flags(&self.into_repr(), writer, flags, 95)
    }

    fn serialized_size(&self) -> usize {
        95
    }
}

impl BeDeserialize for MNT4Fq {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, Error> {
        let value: BigInteger768 = CustomLengthDeSerialize::deserialize(reader, 95)?;
        Ok(MNT4Fq::from_repr(value))
    }

    fn deserialize_with_flags<R: ReadBytesExt>(reader: &mut R) -> Result<(Self, Flags), Error> {
        let (value, flags): (BigInteger768, _) =
            CustomLengthDeSerialize::deserialize_with_flags(reader, 95)?;
        Ok((MNT4Fq::from_repr(value), flags))
    }
}

// MNT4 Fq2
impl BeSerialize for MNT4Fq2 {
    fn serialize_with_flags<W: WriteBytesExt>(
        &self,
        writer: &mut W,
        flags: Flags,
    ) -> Result<(), Error> {
        BeSerialize::serialize_with_flags(&self.c0, writer, flags)?;
        BeSerialize::serialize(&self.c1, writer)
    }

    fn serialized_size(&self) -> usize {
        BeSerialize::serialized_size(&self.c0) + BeSerialize::serialized_size(&self.c1)
    }
}

impl BeDeserialize for MNT4Fq2 {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, Error> {
        let c0: MNT4Fq = BeDeserialize::deserialize(reader)?;
        let c1: MNT4Fq = BeDeserialize::deserialize(reader)?;
        Ok(MNT4Fq2::new(c0, c1))
    }

    fn deserialize_with_flags<R: ReadBytesExt>(reader: &mut R) -> Result<(Self, Flags), Error> {
        let (c0, flags): (MNT4Fq, Flags) = BeDeserialize::deserialize_with_flags(reader)?;
        let c1: MNT4Fq = BeDeserialize::deserialize(reader)?;
        Ok((MNT4Fq2::new(c0, c1), flags))
    }
}

// BigInteger768
impl CustomLengthDeSerialize for BigInteger768 {
    fn serialize_with_flags<W: WriteBytesExt>(
        &self,
        writer: &mut W,
        flags: Flags,
        length: usize,
    ) -> Result<(), Error> {
        // Calculate number of limbs required to pack `length` bytes: ceil(length / 8).
        let num_limbs = (length + 7) / 8;

        // Constraint length.
        if num_limbs > self.0.len() {
            return Err(Error::from(ErrorKind::InvalidInput));
        }

        // Calculate the bit offset, i.e., the number of most significant bytes to skip in the first
        // limb.
        let byte_offset = (num_limbs * 8 - length) % 8;

        // BigInteger stores the least significant u64 first, so we need to reverse the order.
        // Only take num_limbs limbs.
        for (i, &limb) in self.as_ref().iter().take(num_limbs).rev().enumerate() {
            let mut limb_to_encode = limb;
            // Encode flags into very first u64 at most significant bits.
            if i == 0 {
                limb_to_encode |= flags.u64_bitmask(byte_offset as u64 * 8);
                write_u64::<_, BigEndian>(writer, limb_to_encode, byte_offset)?;
            } else {
                writer.write_u64::<BigEndian>(limb_to_encode)?;
            }
        }
        Ok(())
    }

    fn deserialize<R: ReadBytesExt>(reader: &mut R, length: usize) -> Result<Self, Error> {
        // Calculate number of limbs required to pack `length` bytes: ceil(length / 8).
        let num_limbs = (length + 7) / 8;

        // Constraint length.
        if num_limbs > 12 {
            return Err(Error::from(ErrorKind::InvalidInput));
        }

        // Calculate the bit offset, i.e., the number of most significant bytes to skip in the first
        // limb.
        let byte_offset = (num_limbs * 8 - length) % 8;

        let mut limbs = [0u64; 12];
        for (i, limb) in limbs.iter_mut().take(num_limbs).rev().enumerate() {
            if i == 0 {
                *limb = read_u64::<_, BigEndian>(reader, byte_offset)?;
            } else {
                *limb = reader.read_u64::<BigEndian>()?;
            }
        }
        Ok(BigInteger768::new(limbs))
    }

    fn deserialize_with_flags<R: ReadBytesExt>(
        reader: &mut R,
        length: usize,
    ) -> Result<(Self, Flags), Error> {
        // Calculate number of limbs required to pack `length` bytes: ceil(length / 8).
        let num_limbs = (length + 7) / 8;

        // Constraint length.
        if num_limbs > 12 {
            return Err(Error::from(ErrorKind::InvalidInput));
        }

        // Calculate the bit offset, i.e., the number of most significant bytes to skip in the first
        // limb.
        let byte_offset = (num_limbs * 8 - length) % 8;

        let mut limbs = [0u64; 12];
        let mut flags = Default::default();
        for (i, limb) in limbs.iter_mut().take(num_limbs).rev().enumerate() {
            // The first limb encodes the flags, so remove them.
            if i == 0 {
                *limb = read_u64::<_, BigEndian>(reader, byte_offset)?;
                flags = Flags::from_u64_remove_flags(limb, byte_offset as u64 * 8);
            } else {
                *limb = reader.read_u64::<BigEndian>()?;
            }
        }
        Ok((BigInteger768::new(limbs), flags))
    }
}

/// Reads a maximum of `max_length` bytes from the reader, padding it with 0s
/// to construct a u64.
#[inline]
fn read_u64<R: ReadBytesExt, T: ByteOrder>(
    reader: &mut R,
    byte_offset: usize,
) -> Result<u64, Error> {
    let mut buf = [0; 8];
    reader.read_exact(&mut buf[byte_offset..])?;
    Ok(T::read_u64(&buf))
}

/// Reads a maximum of `max_length` bytes from the reader, padding it with 0s
/// to construct a u64.
#[inline]
fn write_u64<W: WriteBytesExt, T: ByteOrder>(
    writer: &mut W,
    n: u64,
    byte_offset: usize,
) -> Result<(), Error> {
    let mut buf = [0; 8];
    T::write_u64(&mut buf, n);
    writer.write_all(&buf[byte_offset..])
}
