extern crate byteorder;
extern crate num;

pub use byteorder::{BigEndian, ByteOrder, ReadBytesExt, WriteBytesExt};
pub use num::ToPrimitive;
use std::io::Result;
pub use types::uvar;

mod types;

// Base traits

pub trait Deserialize: Sized {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self>;

    fn deserialize_from_vec(v: &Vec<u8>) -> Result<Self> {
        return Self::deserialize(&mut &v[..]);
    }
}

pub trait Serialize {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize>;
    fn serialized_size(&self) -> usize;

    fn serialize_to_vec(&self) -> Vec<u8> {
        let mut v = Vec::with_capacity(self.serialized_size());
        self.serialize(&mut v).unwrap();
        return v;
    }
}

// Implementation for u8 and u16/32/64 (big endian)

impl Deserialize for u8 {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self> {
        return reader.read_u8();
    }
}

impl Serialize for u8 {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize> {
        writer.write_u8(*self)?;
        return Ok(self.serialized_size());
    }

    fn serialized_size(&self) -> usize {
        return 1;
    }
}

macro_rules! primitive_serialize {
    ($t: ty, $len: expr, $r: ident, $w: ident) => {
        impl Deserialize for $t {
            fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self> {
                return reader.$r::<BigEndian>();
            }
        }

        impl Serialize for $t {
            fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize> {
                writer.$w::<BigEndian>(*self)?;
                return Ok(self.serialized_size());
            }

            fn serialized_size(&self) -> usize {
                return $len;
            }
        }
    };
}

primitive_serialize!(u16, 2, read_u16, write_u16);
primitive_serialize!(u32, 4, read_u32, write_u32);
primitive_serialize!(u64, 8, read_u64, write_u64);


// Implementation for i8 and i16/32/64 (big endian)

impl Deserialize for i8 {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self> {
        return reader.read_i8();
    }
}

impl Serialize for i8 {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize> {
        writer.write_i8(*self)?;
        return Ok(self.serialized_size());
    }

    fn serialized_size(&self) -> usize {
        return 1;
    }
}

primitive_serialize!(i16, 2, read_i16, write_i16);
primitive_serialize!(i32, 4, read_i32, write_i32);
primitive_serialize!(i64, 8, read_i64, write_i64);


// Boolean

impl Deserialize for bool {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self> {
        return Ok(reader.read_u8()? != 0);
    }
}

impl Serialize for bool {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize> {
        writer.write_u8(if *self { 1u8 } else { 0u8 })?;
        return Ok(self.serialized_size());
    }

    fn serialized_size(&self) -> usize {
        return 1;
    }
}


// String

impl DeserializeWithLength for String {
    fn deserialize<D: Deserialize + num::ToPrimitive, R: ReadBytesExt>(reader: &mut R) -> Result<Self> {
        let vec: Vec<u8> = DeserializeWithLength::deserialize::<D, R>(reader)?;
        match String::from_utf8(vec) {
            Ok(s) => return Ok(s),
            Err(e) => return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, e))
        };
    }
}

impl SerializeWithLength for String {
    fn serialize<S: Serialize + num::FromPrimitive, W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize> {
        return self.as_bytes().to_vec().serialize::<S, W>(writer);
    }

    fn serialized_size<S: Serialize + num::FromPrimitive>(&self) -> usize {
        return self.as_bytes().to_vec().serialized_size::<S>();
    }
}

// Vectors

pub trait DeserializeWithLength: Sized {
    fn deserialize<D: Deserialize + num::ToPrimitive, R: ReadBytesExt>(reader: &mut R) -> Result<Self>;
}

pub trait SerializeWithLength {
    fn serialize<S: Serialize + num::FromPrimitive, W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize>;
    fn serialized_size<S: Serialize + num::FromPrimitive>(&self) -> usize;
    fn serialize_to_vec<S: Serialize + num::FromPrimitive>(&self) -> Vec<u8> {
        let mut v = Vec::with_capacity(self.serialized_size::<S>());
        self.serialize::<S, Vec<u8>>(&mut v).unwrap();
        return v;
    }
}

impl<T: Deserialize> DeserializeWithLength for Vec<T> {
    fn deserialize<D: Deserialize + num::ToPrimitive, R: ReadBytesExt>(reader: &mut R) -> Result<Self> {
        let len: D = Deserialize::deserialize(reader)?;
        let len_u = len.to_usize().unwrap();
        let mut v = Vec::with_capacity(len_u);
        for _ in 0..len_u {
            v.push(T::deserialize(reader)?);
        }
        return Ok(v);
    }
}

impl<T: Serialize> SerializeWithLength for Vec<T> {
    fn serialize<S: Serialize + num::FromPrimitive, W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize> {
        let mut size = S::from_usize(self.len()).unwrap().serialize(writer)?;
        for t in self {
            size += t.serialize(writer)?;
        }
        return Ok(size);
    }

    fn serialized_size<S: Serialize + num::FromPrimitive>(&self) -> usize {
        let mut size = S::from_usize(self.len()).unwrap().serialized_size();
        for t in self {
            size += t.serialized_size();
        }
        return size;
    }
}

// References

impl<'a, T: Serialize> Serialize for &'a T {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize> {
        return Serialize::serialize(*self, writer);
    }

    fn serialized_size(&self) -> usize {
        return Serialize::serialized_size(*self);
    }
}

impl<T: Deserialize> Deserialize for Option<T> {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self> {
        let is_present: u8 = Deserialize::deserialize(reader)?;
        return match is_present {
            0 => Ok(Option::None),
            1 => Ok(Option::Some(Deserialize::deserialize(reader)?)),
            _ => Err(std::io::Error::from(std::io::ErrorKind::InvalidInput))
        };
    }
}

impl<T: Serialize> Serialize for Option<T> {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize> {
        return match self {
            Option::Some(one) => {
                1u8.serialize(writer)?;
                Ok(one.serialize(writer)? + 1)
            }
            Option::None => {
                0u8.serialize(writer)
            }
        };
    }

    fn serialized_size(&self) -> usize {
        return match self {
            Option::Some(one) => 1 + Serialize::serialized_size(one),
            Option::None => 1
        };
    }
}
