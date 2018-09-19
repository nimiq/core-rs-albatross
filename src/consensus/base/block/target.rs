use beserial::{Serialize, Deserialize, SerializeWithLength, DeserializeWithLength, WriteBytesExt, ReadBytesExt};
use bigdecimal::BigDecimal;
use num_bigint::{BigInt, Sign};
use consensus::base::primitive::hash::Argon2dHash;
use consensus::policy;
use std::ops::{Add, Sub};
use std::io;
use std::fmt;

#[derive(Default, Clone, Copy, PartialEq, PartialOrd, Eq, Ord, Debug, Serialize, Deserialize)]
pub struct TargetCompact(u32);

create_typed_array!(Target, u8, 32);

#[derive(Default, Clone, PartialEq, PartialOrd, Eq, Ord, Debug)]
pub struct Difficulty(BigDecimal);

impl From<TargetCompact> for u32 {
    fn from(t: TargetCompact) -> Self { t.0 }
}

impl From<u32> for TargetCompact {
    fn from(u: u32) -> Self { TargetCompact(u) }
}

impl From<TargetCompact> for Target {
    fn from(t: TargetCompact) -> Self {
        let mut val = [0u8; 32];
        let shift_bytes: usize = ((t.0 >> 24) - 3) as usize;
        let value = t.0 & 0xffffff;
        val[32 - shift_bytes - 1] = (value & 0xff) as u8;
        val[32 - shift_bytes - 2] = ((value >> 8) & 0xff) as u8;
        val[32 - shift_bytes - 3] = ((value >> 16) & 0xff) as u8;
        return Target(val);
    }
}

impl From<Target> for TargetCompact {
    fn from(target: Target) -> Self {
        let mut first_byte = 0;
        for i in 0..target.0.len() {
            if target.0[i] > 0 {
                first_byte = i;
                break;
            }
        }
        if target.0[first_byte] >= 0x80 {
            first_byte -= 1;
        }
        let shift_bytes = 32 - first_byte;

        return TargetCompact(((shift_bytes as u32) << 24) + ((target.0[first_byte] as u32) << 16) + ((target.0[first_byte + 1] as u32) << 8) + target.0[first_byte + 2] as u32);
    }
}

impl From<TargetCompact> for Difficulty {
    fn from(t: TargetCompact) -> Self {
        Target::from(t).into()
    }
}

impl From<Target> for Difficulty {
    fn from(target: Target) -> Self {
        Difficulty(&*policy::BLOCK_TARGET_MAX / BigDecimal::parse_bytes(target.as_bytes(), 10).unwrap())
    }
}

impl Add<Difficulty> for Difficulty {
    type Output = Difficulty;

    fn add(self, rhs: Difficulty) -> Difficulty {
        Difficulty(self.0 + rhs.0)
    }
}

impl Sub<Difficulty> for Difficulty {
    type Output = Difficulty;

    fn sub(self, rhs: Difficulty) -> Difficulty {
        Difficulty(self.0 - rhs.0)
    }
}

impl<'a, 'b> Add<&'b Difficulty> for &'a Difficulty {
    type Output = Difficulty;

    fn add(self, rhs: &'b Difficulty) -> Difficulty {
        Difficulty(&self.0 + &rhs.0)
    }
}

impl<'a, 'b> Sub<&'b Difficulty> for &'a Difficulty {
    type Output = Difficulty;

    fn sub(self, rhs: &'b Difficulty) -> Difficulty {
        Difficulty(&self.0 - &rhs.0)
    }
}

impl Serialize for Difficulty {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> io::Result<usize> {
        let (digits, scale) = self.0.as_bigint_and_exponent();
        let (_, bytes) = digits.to_bytes_be();

        let mut size = 0;
        size += SerializeWithLength::serialize::<u8, W>(&bytes, writer)?;
        size += Serialize::serialize(&scale, writer)?;
        return Ok(size);
    }

    fn serialized_size(&self) -> usize {
        let (digits, scale) = self.0.as_bigint_and_exponent();
        let mut size = 1 /*length*/;
        // XXX This assumes that digits.bits() > 0
        size += (digits.bits() - 1) / 8 + 1;
        size += Serialize::serialized_size(&scale);
        return size;
    }
}

impl Deserialize for Difficulty {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> io::Result<Self> {
        let bytes: Vec<u8> = DeserializeWithLength::deserialize::<u8, R>(reader)?;
        let digits: BigInt = BigInt::from_bytes_be(Sign::Plus, bytes.as_slice());
        let scale: i64 = Deserialize::deserialize(reader)?;
        return Ok(Difficulty(BigDecimal::new(digits, scale)));
    }
}

impl fmt::Display for Difficulty {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        return write!(f, "{}", self.0);
    }
}


impl Target {
    pub fn is_met_by(&self, hash: &Argon2dHash) -> bool {
        let reached = Target::from(hash.as_bytes());
        return &reached < self;
    }
}
