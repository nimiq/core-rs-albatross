use std::collections::btree_map::BTreeMap;
use std::collections::HashSet;
use std::hash::BuildHasher;
use std::ops::Deref;

pub use byteorder::{BigEndian, ByteOrder, ReadBytesExt, WriteBytesExt};
use failure::Fail;
pub use num::{FromPrimitive, ToPrimitive};

pub use crate::types::uvar;
use std::sync::Arc;

mod types;
#[cfg(feature = "bitvec")]
mod bitvec;
#[cfg(feature = "net")]
mod net;


// Base traits

pub trait Deserialize: Sized {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError>;

    fn deserialize_from_vec(v: &[u8]) -> Result<Self, SerializingError> {
        Self::deserialize(&mut &v[..])
    }
}

pub trait Serialize {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError>;
    fn serialized_size(&self) -> usize;

    fn serialize_to_vec(&self) -> Vec<u8> {
        let mut v = Vec::with_capacity(self.serialized_size());
        self.serialize(&mut v).unwrap();
        v
    }
}

// Error and result

#[derive(Fail, Debug, PartialEq, Eq, Clone)]
pub enum SerializingError {
    #[fail(display = "IoError(kind={:?}, description={})", _0, _1)]
    IoError(std::io::ErrorKind, String),
    #[fail(display = "Invalid encoding")]
    InvalidEncoding,
    #[fail(display = "Invalid value")]
    InvalidValue,
    #[fail(display = "Overflow")]
    Overflow,
    #[fail(display = "Length limit exceeded")]
    LimitExceeded,
}

impl From<std::io::Error> for SerializingError {
    fn from(io_error: std::io::Error) -> Self {
        SerializingError::IoError(io_error.kind(), format!("{}", io_error))
    }
}

// TODO: Remove
#[deprecated]
impl From<SerializingError> for std::io::Error {
    fn from(_: SerializingError) -> Self {
        std::io::Error::from(std::io::ErrorKind::Other)
    }
}

// Implementation for u8 and u16/32/64 (big endian)

impl Deserialize for u8 {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
        Ok(reader.read_u8()?)
    }
}

impl Serialize for u8 {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        writer.write_u8(*self)?;
        Ok(self.serialized_size())
    }

    fn serialized_size(&self) -> usize {
        1
    }
}

macro_rules! primitive_serialize {
    ($t: ty, $len: expr, $r: ident, $w: ident) => {
        impl Deserialize for $t {
            fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
                Ok(reader.$r::<BigEndian>()?)
            }
        }

        impl Serialize for $t {
            fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
                writer.$w::<BigEndian>(*self)?;
                Ok(self.serialized_size())
            }

            fn serialized_size(&self) -> usize {
                $len
            }
        }
    };
}

primitive_serialize!(u16, 2, read_u16, write_u16);
primitive_serialize!(u32, 4, read_u32, write_u32);
primitive_serialize!(u64, 8, read_u64, write_u64);


// Implementation for i8 and i16/32/64 (big endian)

impl Deserialize for i8 {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
        Ok(reader.read_i8()?)
    }
}

impl Serialize for i8 {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        writer.write_i8(*self)?;
        Ok(self.serialized_size())
    }

    fn serialized_size(&self) -> usize {
        1
    }
}

primitive_serialize!(i16, 2, read_i16, write_i16);
primitive_serialize!(i32, 4, read_i32, write_i32);
primitive_serialize!(i64, 8, read_i64, write_i64);

// Unit

impl Deserialize for () {
    fn deserialize<R: ReadBytesExt>(_reader: &mut R) -> Result<Self, SerializingError> { Ok(()) }
}

impl Serialize for () {
    fn serialize<W: WriteBytesExt>(&self, _writer: &mut W) -> Result<usize, SerializingError> { Ok(0) }

    fn serialized_size(&self) -> usize { 0 }
}


// Boolean

impl Deserialize for bool {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
        match reader.read_u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(SerializingError::InvalidValue),
        }
    }
}

impl Serialize for bool {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        writer.write_u8(if *self { 1u8 } else { 0u8 })?;
        Ok(self.serialized_size())
    }

    fn serialized_size(&self) -> usize {
        1
    }
}


// String

impl DeserializeWithLength for String {
    fn deserialize_with_limit<D: Deserialize + num::ToPrimitive, R: ReadBytesExt>(reader: &mut R, limit: Option<usize>) -> Result<Self, SerializingError> {
        let vec: Vec<u8> = DeserializeWithLength::deserialize_with_limit::<D, R>(reader, limit)?;
        String::from_utf8(vec).or(Err(SerializingError::InvalidEncoding))
    }
}

impl SerializeWithLength for String {
    fn serialize<S: Serialize + num::FromPrimitive, W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        self.as_bytes().to_vec().serialize::<S, W>(writer)
    }

    fn serialized_size<S: Serialize + num::FromPrimitive>(&self) -> usize {
        self.as_bytes().to_vec().serialized_size::<S>()
    }
}

// Vectors

pub trait DeserializeWithLength: Sized {
    fn deserialize<D: Deserialize + num::ToPrimitive, R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
        Self::deserialize_with_limit::<D, _>(reader, None)
    }
    fn deserialize_from_vec<D: Deserialize + num::ToPrimitive>(v: &[u8]) -> Result<Self, SerializingError> {
        Self::deserialize::<D, _>(&mut &v[..])
    }
    fn deserialize_with_limit<D: Deserialize + num::ToPrimitive, R: ReadBytesExt>(reader: &mut R, limit: Option<usize>) -> Result<Self, SerializingError>;
}

pub trait SerializeWithLength {
    fn serialize<S: Serialize + num::FromPrimitive, W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError>;
    fn serialized_size<S: Serialize + num::FromPrimitive>(&self) -> usize;
    fn serialize_to_vec<S: Serialize + num::FromPrimitive>(&self) -> Vec<u8> {
        let mut v = Vec::with_capacity(self.serialized_size::<S>());
        self.serialize::<S, Vec<u8>>(&mut v).unwrap();
        v
    }
}

impl<T: Deserialize> DeserializeWithLength for Vec<T> {
    fn deserialize_with_limit<D: Deserialize + num::ToPrimitive, R: ReadBytesExt>(reader: &mut R, limit: Option<usize>) -> Result<Self, SerializingError> {
        let len: D = Deserialize::deserialize(reader)?;
        let len_u = len.to_usize().unwrap();

        // If vector is too large, abort.
        if limit.map(|l| len_u > l).unwrap_or(false) {
            return Err(SerializingError::LimitExceeded);
        }

        let mut v = Vec::with_capacity(len_u);
        for _ in 0..len_u {
            v.push(T::deserialize(reader)?);
        }
        Ok(v)
    }
}

impl<T: Serialize> SerializeWithLength for Vec<T> {
    fn serialize<S: Serialize + num::FromPrimitive, W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        let mut size = S::from_usize(self.len()).unwrap().serialize(writer)?;
        for t in self {
            size += t.serialize(writer)?;
        }
        Ok(size)
    }

    fn serialized_size<S: Serialize + num::FromPrimitive>(&self) -> usize {
        let mut size = S::from_usize(self.len()).unwrap().serialized_size();
        for t in self {
            size += t.serialized_size();
        }
        size
    }
}

// Box

impl<T: Deserialize> Deserialize for Box<T> {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
        Ok(Box::new(T::deserialize(reader)?))
    }
}

impl<T: Serialize> Serialize for Box<T> {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        Ok(T::serialize(self.deref(), writer)?)
    }

    fn serialized_size(&self) -> usize {
        self.deref().serialized_size()
    }
}

// Arc

impl<T: Deserialize> Deserialize for Arc<T> {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
        Ok(Arc::new(T::deserialize(reader)?))
    }
}

impl<T: Serialize> Serialize for Arc<T> {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        Ok(T::serialize(self.deref(), writer)?)
    }

    fn serialized_size(&self) -> usize {
        self.deref().serialized_size()
    }
}

// HashSets

impl<T, H> DeserializeWithLength for HashSet<T, H>
    where T: Deserialize + std::cmp::Eq + std::hash::Hash,
          H: BuildHasher + Default
{
    fn deserialize_with_limit<D: Deserialize + num::ToPrimitive, R: ReadBytesExt>(reader: &mut R, limit: Option<usize>) -> Result<Self, SerializingError> {
        let len: D = Deserialize::deserialize(reader)?;
        let len_u = len.to_usize().unwrap();

        // If hash set is too large, abort.
        if limit.map(|l| len_u > l).unwrap_or(false) {
            return Err(SerializingError::LimitExceeded);
        }

        let mut v = HashSet::with_capacity_and_hasher(len_u, H::default());
        for _ in 0..len_u {
            v.insert(T::deserialize(reader)?);
        }
        Ok(v)
    }
}

impl<T, H> SerializeWithLength for HashSet<T, H>
    where T: Serialize + std::cmp::Eq + std::hash::Hash,
          H: BuildHasher
{
    fn serialize<S: Serialize + num::FromPrimitive, W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        let mut size = S::from_usize(self.len()).unwrap().serialize(writer)?;
        for t in self {
            size += t.serialize(writer)?;
        }
        Ok(size)
    }

    fn serialized_size<S: Serialize + num::FromPrimitive>(&self) -> usize {
        let mut size = S::from_usize(self.len()).unwrap().serialized_size();
        for t in self {
            size += t.serialized_size();
        }
        size
    }
}

// BTreeMaps

impl<K, V> DeserializeWithLength for BTreeMap<K, V>
    where K: Deserialize + Ord,
        V: Deserialize
{
    fn deserialize_with_limit<D: Deserialize + num::ToPrimitive, R: ReadBytesExt>(reader: &mut R, limit: Option<usize>) -> Result<Self, SerializingError> {
        let len: D = Deserialize::deserialize(reader)?;
        let len_u = len.to_usize().unwrap();

        // If number of items is too large, abort.
        if limit.map(|l| len_u > l).unwrap_or(false) {
            return Err(SerializingError::LimitExceeded);
        }

        let mut v = BTreeMap::new();
        for _ in 0..len_u {
            v.insert(K::deserialize(reader)?, V::deserialize(reader)?);
        }
        Ok(v)
    }
}

impl<K, V> SerializeWithLength for BTreeMap<K, V>
    where K: Serialize + Ord,
          V: Serialize
{
    fn serialize<S: Serialize + num::FromPrimitive, W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        let mut size = S::from_usize(self.len()).unwrap().serialize(writer)?;
        for (k, v) in self {
            size += k.serialize(writer)?;
            size += v.serialize(writer)?;
        }
        Ok(size)
    }

    fn serialized_size<S: Serialize + num::FromPrimitive>(&self) -> usize {
        let mut size = S::from_usize(self.len()).unwrap().serialized_size();
        for (k, v) in self {
            size += k.serialized_size();
            size += v.serialized_size();
        }
        size
    }
}

// References

impl<'a, T: Serialize> Serialize for &'a T {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        Serialize::serialize(*self, writer)
    }

    fn serialized_size(&self) -> usize {
        Serialize::serialized_size(*self)
    }
}

impl<T: Deserialize> Deserialize for Option<T> {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
        let is_present: u8 = Deserialize::deserialize(reader)?;
        match is_present {
            0 => Ok(Option::None),
            1 => Ok(Option::Some(Deserialize::deserialize(reader)?)),
            _ => Err(SerializingError::InvalidValue),
        }
    }
}

impl<T: DeserializeWithLength> DeserializeWithLength for Option<T> {
    fn deserialize_with_limit<D: Deserialize + num::ToPrimitive, R: ReadBytesExt>(reader: &mut R, limit: Option<usize>) -> Result<Self, SerializingError> {
        let is_present: u8 = Deserialize::deserialize(reader)?;
        match is_present {
            0 => Ok(Option::None),
            1 => Ok(Option::Some(DeserializeWithLength::deserialize_with_limit::<D, R>(reader, limit)?)),
            _ => Err(SerializingError::InvalidValue),
        }
    }
}

impl<T: Serialize> Serialize for Option<T> {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        match self {
            Option::Some(one) => {
                1u8.serialize(writer)?;
                Ok(one.serialize(writer)? + 1)
            }
            Option::None => {
                0u8.serialize(writer)
            }
        }
    }

    fn serialized_size(&self) -> usize {
        match self {
            Option::Some(one) => 1 + Serialize::serialized_size(one),
            Option::None => 1
        }
    }
}

impl<T: SerializeWithLength> SerializeWithLength for Option<T> {
    fn serialize<S: Serialize + num::FromPrimitive, W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        match self {
            Option::Some(one) => {
                1u8.serialize(writer)?;
                Ok(SerializeWithLength::serialize::<S, W>(one, writer)? + 1)
            }
            Option::None => {
                0u8.serialize(writer)
            }
        }
    }

    fn serialized_size<S: Serialize + num::FromPrimitive>(&self) -> usize {
        match self {
            Option::Some(one) => 1 + SerializeWithLength::serialized_size::<S>(one),
            Option::None => 1
        }
    }
}
