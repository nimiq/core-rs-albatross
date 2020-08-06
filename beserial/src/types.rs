use crate::{Deserialize, ReadBytesExt, Serialize, SerializingError, WriteBytesExt};

#[allow(non_camel_case_types)]
#[derive(Ord, PartialOrd, Eq, PartialEq, Debug, Copy, Clone)]
pub struct uvar(u64);

impl uvar {
    pub fn serialized_size_from_first_byte(first_byte: u8) -> usize {
        if first_byte == 0xFF {
            9
        } else if first_byte == 0xFE {
            8
        } else if first_byte & 0xFC == 0xFC {
            7
        } else if first_byte & 0xF8 == 0xF8 {
            6
        } else if first_byte & 0xF0 == 0xF0 {
            5
        } else if first_byte & 0xE0 == 0xE0 {
            4
        } else if first_byte & 0xC0 == 0xC0 {
            3
        } else if first_byte & 0x80 == 0x80 {
            2
        } else {
            1
        }
    }
}

impl From<uvar> for u64 {
    fn from(u: uvar) -> Self {
        u.0
    }
}

impl From<u64> for uvar {
    fn from(u: u64) -> Self {
        uvar(u)
    }
}

impl num::FromPrimitive for uvar {
    fn from_i64(n: i64) -> Option<Self> {
        if n < 0 {
            None
        } else {
            Some(uvar(n as u64))
        }
    }

    fn from_u64(n: u64) -> Option<Self> {
        Some(uvar(n))
    }
}

impl num::ToPrimitive for uvar {
    fn to_i64(&self) -> Option<i64> {
        if self.0 > i64::max_value() as u64 {
            None
        } else {
            Some(self.0 as i64)
        }
    }

    fn to_u64(&self) -> Option<u64> {
        Some(self.0)
    }
}

impl Serialize for uvar {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        let mut size = 0;
        if self.0 < 0x80 {
            // Just that byte
            size += Serialize::serialize(&(self.0 as u8), writer)?;
        } else if self.0 < 0x4080 {
            // +1 bytes
            let x = self.0 - 0x80;
            size += Serialize::serialize(&((x | 0x8000) as u16), writer)?;
        } else if self.0 < 0x0020_4080 {
            // +2 bytes
            let x = self.0 - 0x4080;
            size += Serialize::serialize(&(((x >> 8) | 0xC000) as u16), writer)?;
            size += Serialize::serialize(&((x & 0xFF) as u8), writer)?;
        } else if self.0 < 0x1020_4080 {
            // +3 bytes
            let x = self.0 - 0x0020_4080;
            size += Serialize::serialize(&((x | 0xE000_0000) as u32), writer)?;
        } else if self.0 < 0x0008_1020_4080 {
            // +4 bytes
            let x = self.0 - 0x1020_4080;
            size += Serialize::serialize(&(((x >> 8) | 0xF000_0000) as u32), writer)?;
            size += Serialize::serialize(&((x & 0xFF) as u8), writer)?;
        } else if self.0 < 0x0408_1020_4080 {
            // +5 bytes
            let x = self.0 - 0x0008_1020_4080;
            size += Serialize::serialize(&(((x >> 16) | 0xF800_0000) as u32), writer)?;
            size += Serialize::serialize(&((x & 0xFFFF) as u16), writer)?;
        } else if self.0 < 0x0002_0408_1020_4080 {
            // +6 bytes
            let x = self.0 - 0x0408_1020_4080;
            size += Serialize::serialize(&(((x >> 24) | 0xFC00_0000) as u32), writer)?;
            size += Serialize::serialize(&(((x >> 8) & 0xFFFF) as u16), writer)?;
            size += Serialize::serialize(&((x & 0xFF) as u8), writer)?;
        } else if self.0 < 0x0102_0408_1020_4080 {
            // +7 bytes
            let x = self.0 - 0x0002_0408_1020_4080;
            size += Serialize::serialize(&((x | 0xFE00_0000_0000_0000) as u64), writer)?;
        } else {
            // +8 bytes
            let x = self.0 - 0x0102_0408_1020_4080;
            size += Serialize::serialize(&(((x >> 8) | 0xFF00_0000_0000_0000) as u64), writer)?;
            size += Serialize::serialize(&((x & 0xFF) as u8), writer)?;
        }
        Ok(size)
    }

    fn serialized_size(&self) -> usize {
        if self.0 < 0x80 {
            1
        } else if self.0 < 0x4080 {
            2
        } else if self.0 < 0x0020_4080 {
            3
        } else if self.0 < 0x1020_4080 {
            4
        } else if self.0 < 0x0008_1020_4080 {
            5
        } else if self.0 < 0x0408_1020_4080 {
            6
        } else if self.0 < 0x0002_0408_1020_4080 {
            7
        } else if self.0 < 0x0102_0408_1020_4080 {
            8
        } else {
            9
        }
    }
}

impl Deserialize for uvar {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
        fn read<T: num::ToPrimitive + Deserialize, R: ReadBytesExt>(reader: &mut R) -> Result<u64, SerializingError> {
            let n: T = Deserialize::deserialize(reader)?;
            Ok(n.to_u64().unwrap())
        }
        let first_byte: u8 = Deserialize::deserialize(reader)?;
        if first_byte == 0xFF {
            // 8 bytes follow
            let byte_1_8 = read::<u64, R>(reader)?;
            if byte_1_8 > u64::max_value() - 0x0102_0408_1020_4080 {
                return Err(SerializingError::Overflow);
            }
            Ok(uvar(byte_1_8 + 0x0102_0408_1020_4080))
        } else if first_byte == 0xFE {
            // 7 bytes follow
            let byte_1 = read::<u8, R>(reader)?;
            let byte_2_3 = read::<u16, R>(reader)?;
            let byte_4_7 = read::<u32, R>(reader)?;
            Ok(uvar((byte_1 << 48) + (byte_2_3 << 32) + byte_4_7 + 0x0002_0408_1020_4080))
        } else if first_byte & 0xFC == 0xFC {
            // 6 bytes follow
            let byte_1_2 = read::<u16, R>(reader)?;
            let byte_3_6 = read::<u32, R>(reader)?;
            Ok(uvar(((u64::from(first_byte) & 0x01) << 48) + (byte_1_2 << 32) + byte_3_6 + 0x0408_1020_4080))
        } else if first_byte & 0xF8 == 0xF8 {
            // 5 bytes to follow
            let byte_1 = read::<u8, R>(reader)?;
            let byte_2_5 = read::<u32, R>(reader)?;
            Ok(uvar(((u64::from(first_byte) & 0x03) << 40) + (byte_1 << 32) + byte_2_5 + 0x0008_1020_4080))
        } else if first_byte & 0xF0 == 0xF0 {
            // 4 bytes to follow
            let byte_1_4 = read::<u32, R>(reader)?;
            Ok(uvar(((u64::from(first_byte) & 0x07) << 32) + byte_1_4 + 0x1020_4080))
        } else if first_byte & 0xE0 == 0xE0 {
            // 3 bytes to follow
            let byte_1 = read::<u8, R>(reader)?;
            let byte_2_3 = read::<u16, R>(reader)?;
            Ok(uvar(((u64::from(first_byte) & 0x0f) << 24) + (byte_1 << 16) + byte_2_3 + 0x0020_4080))
        } else if first_byte & 0xC0 == 0xC0 {
            // 2 bytes to follow
            let byte_1_2 = read::<u16, R>(reader)?;
            Ok(uvar(((u64::from(first_byte) & 0x1f) << 16) + byte_1_2 + 0x4080))
        } else if first_byte & 0x80 == 0x80 {
            // 1 byte follows
            let byte_1 = read::<u8, R>(reader)?;
            Ok(uvar(((u64::from(first_byte) & 0x3f) << 8) + byte_1 + 0x80))
        } else {
            // Just that byte
            Ok(uvar(u64::from(first_byte)))
        }
    }
}
