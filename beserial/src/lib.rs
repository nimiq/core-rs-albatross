use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::hash::BuildHasher;
use std::ops::Deref;
use std::sync::Arc;

pub use byteorder::{BigEndian, ByteOrder, ReadBytesExt, WriteBytesExt};
pub use num::{FromPrimitive, ToPrimitive};
use thiserror::Error;

pub use crate::types::uvar;

#[cfg(feature = "bitvec")]
mod bitvec;
#[cfg(feature = "libp2p")]
mod libp2p;
#[cfg(feature = "net")]
mod net;
mod types;

// Base traits

pub trait Deserialize: Sized {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError>;

    fn deserialize_from_vec(v: &[u8]) -> Result<Self, SerializingError> {
        Self::deserialize(&mut &*v)
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
#[derive(Error, Debug)]
pub enum SerializingError {
    #[error("IoError: {0}")]
    IoError(#[from] std::io::Error),

    #[error("Invalid encoding")]
    InvalidEncoding,

    #[error("Invalid value")]
    InvalidValue,

    #[error("Overflow")]
    Overflow,

    #[error("Length limit exceeded")]
    LimitExceeded,
}

impl Eq for SerializingError {}

impl PartialEq for SerializingError {
    fn eq(&self, other: &SerializingError) -> bool {
        match (self, other) {
            (Self::IoError(e1), Self::IoError(e2)) if e1.kind() == e2.kind() => true,
            (Self::InvalidEncoding, Self::InvalidEncoding)
            | (Self::InvalidValue, Self::InvalidValue)
            | (Self::Overflow, Self::Overflow)
            | (Self::LimitExceeded, Self::LimitExceeded) => true,
            _ => false,
        }
    }
}

/// # Notes
///
///  - This was marked as deprecated, but deprecation attributes don't work on impls and are now an error.
///  - Why was this marked as deprecated in the first place?
///
impl From<SerializingError> for std::io::Error {
    fn from(e: SerializingError) -> Self {
        match e {
            SerializingError::IoError(io_error) => io_error,
            _ => std::io::Error::from(std::io::ErrorKind::Other),
        }
    }
}

// Implementation for u8 and u16/32/64/128 (big endian)

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
            fn serialize<W: WriteBytesExt>(
                &self,
                writer: &mut W,
            ) -> Result<usize, SerializingError> {
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
primitive_serialize!(u128, 16, read_u128, write_u128);

// Implementation for i8 and i16/32/64/128 (big endian)

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
primitive_serialize!(i128, 16, read_i128, write_i128);

// Implementation for f32/f64

primitive_serialize!(f32, 4, read_f32, write_f32);
primitive_serialize!(f64, 8, read_f64, write_f64);

// Unit

impl Deserialize for () {
    fn deserialize<R: ReadBytesExt>(_reader: &mut R) -> Result<Self, SerializingError> {
        Ok(())
    }
}

impl Serialize for () {
    fn serialize<W: WriteBytesExt>(&self, _writer: &mut W) -> Result<usize, SerializingError> {
        Ok(0)
    }

    fn serialized_size(&self) -> usize {
        0
    }
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
    fn deserialize_with_limit<D: Deserialize + num::ToPrimitive, R: ReadBytesExt>(
        reader: &mut R,
        limit: Option<usize>,
    ) -> Result<Self, SerializingError> {
        let vec: Vec<u8> = DeserializeWithLength::deserialize_with_limit::<D, R>(reader, limit)?;
        String::from_utf8(vec).or(Err(SerializingError::InvalidEncoding))
    }
}

impl SerializeWithLength for String {
    fn serialize<S: Serialize + num::FromPrimitive, W: WriteBytesExt>(
        &self,
        writer: &mut W,
    ) -> Result<usize, SerializingError> {
        self.as_bytes().to_vec().serialize::<S, W>(writer)
    }

    fn serialized_size<S: Serialize + num::FromPrimitive>(&self) -> usize {
        self.as_bytes().to_vec().serialized_size::<S>()
    }
}

// Vectors

pub trait DeserializeWithLength: Sized {
    fn deserialize<D: Deserialize + num::ToPrimitive, R: ReadBytesExt>(
        reader: &mut R,
    ) -> Result<Self, SerializingError> {
        Self::deserialize_with_limit::<D, _>(reader, None)
    }
    fn deserialize_from_vec<D: Deserialize + num::ToPrimitive>(
        v: &[u8],
    ) -> Result<Self, SerializingError> {
        Self::deserialize::<D, _>(&mut &*v)
    }
    fn deserialize_with_limit<D: Deserialize + num::ToPrimitive, R: ReadBytesExt>(
        reader: &mut R,
        limit: Option<usize>,
    ) -> Result<Self, SerializingError>;
}

pub trait SerializeWithLength {
    fn serialize<S: Serialize + num::FromPrimitive, W: WriteBytesExt>(
        &self,
        writer: &mut W,
    ) -> Result<usize, SerializingError>;
    fn serialized_size<S: Serialize + num::FromPrimitive>(&self) -> usize;
    fn serialize_to_vec<S: Serialize + num::FromPrimitive>(&self) -> Vec<u8> {
        let mut v = Vec::with_capacity(self.serialized_size::<S>());
        self.serialize::<S, Vec<u8>>(&mut v).unwrap();
        v
    }
}

impl<T: Deserialize> DeserializeWithLength for Vec<T> {
    fn deserialize_with_limit<D: Deserialize + num::ToPrimitive, R: ReadBytesExt>(
        reader: &mut R,
        limit: Option<usize>,
    ) -> Result<Self, SerializingError> {
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
    fn serialize<S: Serialize + num::FromPrimitive, W: WriteBytesExt>(
        &self,
        writer: &mut W,
    ) -> Result<usize, SerializingError> {
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
        T::serialize(self.deref(), writer)
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
        T::serialize(self.deref(), writer)
    }

    fn serialized_size(&self) -> usize {
        self.deref().serialized_size()
    }
}

// HashMaps

impl<K, V, H> DeserializeWithLength for HashMap<K, V, H>
where
    K: Deserialize + std::cmp::Eq + std::hash::Hash,
    V: Deserialize,
    H: BuildHasher + Default,
{
    fn deserialize_with_limit<D: Deserialize + num::ToPrimitive, R: ReadBytesExt>(
        reader: &mut R,
        limit: Option<usize>,
    ) -> Result<Self, SerializingError> {
        let len: D = Deserialize::deserialize(reader)?;
        let len_u = len.to_usize().unwrap();

        // If hash map is too large, abort.
        if limit.map(|l| len_u > l).unwrap_or(false) {
            return Err(SerializingError::LimitExceeded);
        }

        let mut v = HashMap::with_capacity_and_hasher(len_u, H::default());
        for _ in 0..len_u {
            v.insert(K::deserialize(reader)?, V::deserialize(reader)?);
        }
        Ok(v)
    }
}

impl<K, V, H> SerializeWithLength for HashMap<K, V, H>
where
    K: Serialize + std::cmp::Eq + std::hash::Hash + std::cmp::Ord,
    V: Serialize,
    H: BuildHasher,
{
    fn serialize<S: Serialize + num::FromPrimitive, W: WriteBytesExt>(
        &self,
        writer: &mut W,
    ) -> Result<usize, SerializingError> {
        let mut size = S::from_usize(self.len()).unwrap().serialize(writer)?;

        let mut vec = self.iter().collect::<Vec<(&K, &V)>>();

        vec.sort_unstable_by_key(|t| t.0);

        for (k, v) in vec {
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

// HashSets

impl<T, H> DeserializeWithLength for HashSet<T, H>
where
    T: Deserialize + std::cmp::Eq + std::hash::Hash,
    H: BuildHasher + Default,
{
    fn deserialize_with_limit<D: Deserialize + num::ToPrimitive, R: ReadBytesExt>(
        reader: &mut R,
        limit: Option<usize>,
    ) -> Result<Self, SerializingError> {
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
where
    T: Serialize + std::cmp::Eq + std::hash::Hash + std::cmp::Ord,
    H: BuildHasher,
{
    fn serialize<S: Serialize + num::FromPrimitive, W: WriteBytesExt>(
        &self,
        writer: &mut W,
    ) -> Result<usize, SerializingError> {
        let mut size = S::from_usize(self.len()).unwrap().serialize(writer)?;

        let mut v = self.iter().collect::<Vec<&T>>();
        v.sort_unstable();

        for t in v {
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
where
    K: Deserialize + Ord,
    V: Deserialize,
{
    fn deserialize_with_limit<D: Deserialize + num::ToPrimitive, R: ReadBytesExt>(
        reader: &mut R,
        limit: Option<usize>,
    ) -> Result<Self, SerializingError> {
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
where
    K: Serialize + Ord,
    V: Serialize,
{
    fn serialize<S: Serialize + num::FromPrimitive, W: WriteBytesExt>(
        &self,
        writer: &mut W,
    ) -> Result<usize, SerializingError> {
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

// BTreeSet

impl<V> DeserializeWithLength for BTreeSet<V>
where
    V: Deserialize + Ord,
{
    fn deserialize_with_limit<D: Deserialize + num::ToPrimitive, R: ReadBytesExt>(
        reader: &mut R,
        limit: Option<usize>,
    ) -> Result<Self, SerializingError> {
        let len: D = Deserialize::deserialize(reader)?;
        let len_u = len.to_usize().unwrap();

        // If number of items is too large, abort.
        if limit.map(|l| len_u > l).unwrap_or(false) {
            return Err(SerializingError::LimitExceeded);
        }

        let mut v = BTreeSet::new();
        for _ in 0..len_u {
            v.insert(V::deserialize(reader)?);
        }
        Ok(v)
    }
}

impl<V> SerializeWithLength for BTreeSet<V>
where
    V: Serialize + Ord,
{
    fn serialize<S: Serialize + num::FromPrimitive, W: WriteBytesExt>(
        &self,
        writer: &mut W,
    ) -> Result<usize, SerializingError> {
        let mut size = S::from_usize(self.len()).unwrap().serialize(writer)?;
        for v in self {
            size += v.serialize(writer)?;
        }
        Ok(size)
    }

    fn serialized_size<S: Serialize + num::FromPrimitive>(&self) -> usize {
        let mut size = S::from_usize(self.len()).unwrap().serialized_size();
        for v in self {
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

// Option

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
    fn deserialize_with_limit<D: Deserialize + num::ToPrimitive, R: ReadBytesExt>(
        reader: &mut R,
        limit: Option<usize>,
    ) -> Result<Self, SerializingError> {
        let is_present: u8 = Deserialize::deserialize(reader)?;
        match is_present {
            0 => Ok(Option::None),
            1 => Ok(Option::Some(
                DeserializeWithLength::deserialize_with_limit::<D, R>(reader, limit)?,
            )),
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
            Option::None => 0u8.serialize(writer),
        }
    }

    fn serialized_size(&self) -> usize {
        match self {
            Option::Some(one) => 1 + Serialize::serialized_size(one),
            Option::None => 1,
        }
    }
}

impl<T: SerializeWithLength> SerializeWithLength for Option<T> {
    fn serialize<S: Serialize + num::FromPrimitive, W: WriteBytesExt>(
        &self,
        writer: &mut W,
    ) -> Result<usize, SerializingError> {
        match self {
            Option::Some(one) => {
                1u8.serialize(writer)?;
                Ok(SerializeWithLength::serialize::<S, W>(one, writer)? + 1)
            }
            Option::None => 0u8.serialize(writer),
        }
    }

    fn serialized_size<S: Serialize + num::FromPrimitive>(&self) -> usize {
        match self {
            Option::Some(one) => 1 + SerializeWithLength::serialized_size::<S>(one),
            Option::None => 1,
        }
    }
}

// Tuples
impl<T: Serialize, V: Serialize> Serialize for (T, V) {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        Ok(self.0.serialize(writer)? + self.1.serialize(writer)?)
    }

    fn serialized_size(&self) -> usize {
        self.0.serialized_size() + self.1.serialized_size()
    }
}

impl<T: Deserialize, V: Deserialize> Deserialize for (T, V) {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
        Ok((
            Deserialize::deserialize(reader)?,
            Deserialize::deserialize(reader)?,
        ))
    }
}

// Slices (only serialization)

impl<T: Serialize> SerializeWithLength for [T] {
    fn serialize<S: Serialize + num::FromPrimitive, W: WriteBytesExt>(
        &self,
        writer: &mut W,
    ) -> Result<usize, SerializingError> {
        let mut size = 0;

        let n = S::from_usize(self.len()).ok_or(SerializingError::Overflow)?;
        size += Serialize::serialize(&n, writer)?;

        for elem in self {
            size += Serialize::serialize(elem, writer)?;
        }

        Ok(size)
    }

    fn serialized_size<S: Serialize + num::FromPrimitive>(&self) -> usize {
        let mut size = 0;

        let n = S::from_usize(self.len()).unwrap();
        size += Serialize::serialized_size(&n);

        for elem in self {
            size += Serialize::serialized_size(elem);
        }

        size
    }
}
