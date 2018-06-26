extern crate byteorder;

use std::io::{Read,Write,Result};
pub use byteorder::{BigEndian, ReadBytesExt, WriteBytesExt, ByteOrder};

// Base traits

pub trait Deserialize: Sized {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self>;
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

// Vectors

pub trait Deserialize8: Sized {
    fn deserialize8<R: ReadBytesExt>(reader: &mut R) -> Result<Self>;
}

pub trait Serialize8 {
    fn serialize8<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize>;
    fn serialize8d_size(&self) -> usize;
}

pub trait Serialize16 {
    fn serialize16<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize>;
    fn serialize16d_size(&self) -> usize;
}

pub trait Deserialize16: Sized {
    fn deserialize16<R: ReadBytesExt>(reader: &mut R) -> Result<Self>;
}

impl<T: Deserialize> Deserialize16 for Vec<T> {
    fn deserialize16<R: ReadBytesExt>(reader: &mut R) -> Result<Self> {
        let len: u16 = Deserialize::deserialize(reader)?;
        let mut v = Vec::with_capacity(len as usize);
        for x in 0..len {
            v.push(T::deserialize(reader)?);
        }
        return Ok(v);
    }
}

impl<T: Serialize> Serialize8 for Vec<T> {
    fn serialize8<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize> {
        let mut size = (self.len() as u8).serialize(writer)?;
        for t in self {
            size += t.serialize(writer)?;
        }
        return Ok(size);
    }

    fn serialize8d_size(&self) -> usize {
        let mut size = 1;
        for t in self {
            size += t.serialized_size();
        }
        return size;
    }
}

/*impl Deserialize16 for Vec<u8> {
    fn deserialize_u16<R: ReadBytesExt>(reader: &mut R) -> Result<Self> {
        let len: u16 = Deserialize::deserialize(reader)?;
        let mut v = Vec::with_capacity(len as usize);
        reader.read_exact(&mut v)?;
        return Ok(v);
    }
}*/
