use crate::{Deserialize, ReadBytesExt, Serialize, WriteBytesExt};
use num;
use std::io;

#[allow(non_camel_case_types)]
#[derive(Ord, PartialOrd, Eq, PartialEq, Debug, Copy, Clone)]
pub struct uvar(u64);

impl From<uvar> for u64 {
    fn from(u: uvar) -> Self { u.0 }
}

impl From<u64> for uvar {
    fn from(u: u64) -> Self { uvar(u) }
}

impl num::FromPrimitive for uvar {
    fn from_i64(n: i64) -> Option<Self> { if n < 0 { None } else { Some(uvar(n as u64)) } }

    fn from_u64(n: u64) -> Option<Self> { Some(uvar(n)) }
}

impl num::ToPrimitive for uvar {
    fn to_i64(&self) -> Option<i64> { if self.0 > i64::max_value() as u64 { None } else { Some(self.0 as i64) } }

    fn to_u64(&self) -> Option<u64> { Some(self.0) }
}

impl Serialize for uvar {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> io::Result<usize> {
        let mut size = 0;
        if self.0 < 0x80 {
            // Just that byte
            size += Serialize::serialize(&(self.0 as u8), writer)?;
        } else if self.0 < 0x4080 {
            // +1 bytes
            let x = self.0 - 0x80;
            size += Serialize::serialize(&((x | 0x8000) as u16), writer)?;
        } else if self.0 < 0x204080 {
            // +2 bytes
            let x = self.0 - 0x4080;
            size += Serialize::serialize(&(((x >> 8) | 0xC000) as u16), writer)?;
            size += Serialize::serialize(&((x & 0xFF) as u8), writer)?;
        } else if self.0 < 0x10204080 {
            // +3 bytes
            let x = self.0 - 0x204080;
            size += Serialize::serialize(&((x | 0xE0000000) as u32), writer)?;
        } else if self.0 < 0x0810204080 {
            // +4 bytes
            let x = self.0 - 0x10204080;
            size += Serialize::serialize(&(((x >> 8) | 0xF0000000) as u32), writer)?;
            size += Serialize::serialize(&((x & 0xFF) as u8), writer)?;
        } else if self.0 < 0x040810204080 {
            // +5 bytes
            let x = self.0 - 0x0810204080;
            size += Serialize::serialize(&(((x >> 16) | 0xF8000000) as u32), writer)?;
            size += Serialize::serialize(&((x & 0xFFFF) as u16), writer)?;
        } else if self.0 < 0x02040810204080 {
            // +6 bytes
            let x = self.0 - 0x040810204080;
            size += Serialize::serialize(&(((x >> 24) | 0xFC000000) as u32), writer)?;
            size += Serialize::serialize(&(((x >> 8) & 0xFFFF) as u16), writer)?;
            size += Serialize::serialize(&((x & 0xFF) as u8), writer)?;
        } else if self.0 < 0x0102040810204080 {
            // +7 bytes
            let x = self.0 - 0x02040810204080;
            size += Serialize::serialize(&((x | 0xFE00000000000000) as u64), writer)?;
        } else {
            // +8 bytes
            let x = self.0 - 0x0102040810204080;
            size += Serialize::serialize(&(((x >> 8) | 0xFF00000000000000) as u64), writer)?;
            size += Serialize::serialize(&((x & 0xFF) as u8), writer)?;
        }
        return Ok(size);
    }

    fn serialized_size(&self) -> usize {
        if self.0 < 0x80 {
            return 1;
        } else if self.0 < 0x4080 {
            return 2;
        } else if self.0 < 0x204080 {
            return 3;
        } else if self.0 < 0x10204080 {
            return 4;
        } else if self.0 < 0x0810204080 {
            return 5;
        } else if self.0 < 0x040810204080 {
            return 6;
        } else if self.0 < 0x02040810204080 {
            return 7;
        } else if self.0 < 0x0102040810204080 {
            return 8;
        } else { return 9; }
    }
}

impl Deserialize for uvar {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> io::Result<Self> {
        fn read<T: num::ToPrimitive + Deserialize, R: ReadBytesExt>(reader: &mut R) -> io::Result<u64> {
            let n: T = Deserialize::deserialize(reader)?;
            return n.to_u64().ok_or_else(|| io::Error::from(io::ErrorKind::InvalidInput));
        }
        let first_byte: u8 = Deserialize::deserialize(reader)?;
        if first_byte == 0xFF {
            // 8 bytes follow
            let byte_1_8 = read::<u64, R>(reader)?;
            if byte_1_8 > u64::max_value() - 0x0102040810204080 {
                return Err(io::Error::from(io::ErrorKind::InvalidInput));
            }
            return Ok(uvar(byte_1_8 + 0x0102040810204080));
        } else if first_byte == 0xFE {
            // 7 bytes follow
            let byte_1 = read::<u8, R>(reader)?;
            let byte_2_3 = read::<u16, R>(reader)?;
            let byte_4_7 = read::<u32, R>(reader)?;
            return Ok(uvar((byte_1 << 48) + (byte_2_3 << 32) + byte_4_7 + 0x02040810204080));
        } else if first_byte & 0xFC == 0xFC {
            // 6 bytes follow
            let byte_1_2 = read::<u16, R>(reader)?;
            let byte_3_6 = read::<u32, R>(reader)?;
            return Ok(uvar(((first_byte as u64 & 0x01) << 48) + (byte_1_2 << 32) + byte_3_6 + 0x040810204080));
        } else if first_byte & 0xF8 == 0xF8 {
            // 5 bytes to follow
            let byte_1 = read::<u8, R>(reader)?;
            let byte_2_5 = read::<u32, R>(reader)?;
            return Ok(uvar(((first_byte as u64 & 0x03) << 40) + (byte_1 << 32) + byte_2_5 + 0x0810204080));
        } else if first_byte & 0xF0 == 0xF0 {
            // 4 bytes to follow
            let byte_1_4 = read::<u32, R>(reader)?;
            return Ok(uvar(((first_byte as u64 & 0x07) << 32) + byte_1_4 + 0x10204080));
        } else if first_byte & 0xE0 == 0xE0 {
            // 3 bytes to follow
            let byte_1 = read::<u8, R>(reader)?;
            let byte_2_3 = read::<u16, R>(reader)?;
            return Ok(uvar(((first_byte as u64 & 0x0f) << 24) + (byte_1 << 16) + byte_2_3 + 0x204080));
        } else if first_byte & 0xC0 == 0xC0 {
            // 2 bytes to follow
            let byte_1_2 = read::<u16, R>(reader)?;
            return Ok(uvar(((first_byte as u64 & 0x1f) << 16) + byte_1_2 + 0x4080));
        } else if first_byte & 0x80 == 0x80 {
            // 1 byte follows
            let byte_1 = read::<u8, R>(reader)?;
            return Ok(uvar(((first_byte as u64 & 0x3f) << 8) + byte_1 + 0x80));
        } else {
            // Just that byte
            return Ok(uvar(first_byte as u64));
        }
    }
} 
