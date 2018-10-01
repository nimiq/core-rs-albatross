use beserial::{Serialize, Deserialize, SerializeWithLength, DeserializeWithLength, WriteBytesExt, ReadBytesExt};
use bigdecimal::BigDecimal;
use num_bigint::{BigInt, Sign, ToBigInt};
use consensus::base::primitive::hash::Argon2dHash;
use consensus::policy;
use std::ops::{Add, AddAssign, Sub, SubAssign};
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
        let shift_bytes: usize = ((t.0 >> 24) as i16 - 3).max(0) as usize;
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

        if target.0[first_byte] >= 0x80 && first_byte <= 29 {
            first_byte -= 1;
        }

        let shift_bytes = 32 - first_byte;
        let start_byte = first_byte.min(29);
        return TargetCompact(((shift_bytes as u32) << 24) + ((target.0[start_byte] as u32) << 16) + ((target.0[start_byte + 1] as u32) << 8) + target.0[start_byte + 2] as u32);
    }
}

impl<'a> From<&'a Argon2dHash> for Target {
    fn from(hash: &'a Argon2dHash) -> Self {
        Target::from(hash.as_bytes())
    }
}

impl From<BigDecimal> for Target {
    fn from(decimal: BigDecimal) -> Target {
        let bytes = decimal.to_bigint().unwrap().to_bytes_be().1;
        let byte_len = bytes.len();
        assert!(byte_len <= 32, "Cannot convert BigDecimal to Target - out of bounds");

        let mut target = [0u8; 32];
        for i in 0..byte_len {
            target[32 - byte_len + i] = bytes[i];
        }

        return Target(target);
    }
}

impl From<Target> for BigDecimal {
    fn from(target: Target) -> Self {
        BigDecimal::new(BigInt::from_bytes_be(Sign::Plus, &target.0), 0)
    }
}

impl From<TargetCompact> for Difficulty {
    fn from(t: TargetCompact) -> Self {
        Target::from(t).into()
    }
}

impl From<Difficulty> for TargetCompact {
    fn from(difficulty: Difficulty) -> Self {
        Target::from(difficulty).into()
    }
}

impl From<Target> for Difficulty {
    fn from(target: Target) -> Self {
        Difficulty(&*policy::BLOCK_TARGET_MAX / BigDecimal::from(target))
    }
}

impl From<Difficulty> for Target {
    fn from(difficulty: Difficulty) -> Self {
        Target::from(&*policy::BLOCK_TARGET_MAX / BigDecimal::from(difficulty))
    }
}

impl From<BigDecimal> for Difficulty {
    fn from(decimal: BigDecimal) -> Self { Difficulty(decimal) }
}

impl From<Difficulty> for BigDecimal {
    fn from(difficulty: Difficulty) -> Self { difficulty.0 }
}

impl Add<Difficulty> for Difficulty {
    type Output = Difficulty;

    fn add(self, rhs: Difficulty) -> Difficulty {
        Difficulty(self.0 + rhs.0)
    }
}

impl<'a, 'b> Add<&'b Difficulty> for &'a Difficulty {
    type Output = Difficulty;

    fn add(self, rhs: &'b Difficulty) -> Difficulty {
        Difficulty(&self.0 + &rhs.0)
    }
}

impl AddAssign<Difficulty> for Difficulty {
    fn add_assign(&mut self, rhs: Difficulty) {
        *self = Difficulty(&self.0 + &rhs.0);
    }
}

impl Sub<Difficulty> for Difficulty {
    type Output = Difficulty;

    fn sub(self, rhs: Difficulty) -> Difficulty {
        Difficulty(self.0 - rhs.0)
    }
}

impl<'a, 'b> Sub<&'b Difficulty> for &'a Difficulty {
    type Output = Difficulty;

    fn sub(self, rhs: &'b Difficulty) -> Difficulty {
        Difficulty(&self.0 - &rhs.0)
    }
}

impl SubAssign<Difficulty> for Difficulty {
    fn sub_assign(&mut self, rhs: Difficulty) {
        *self = Difficulty(&self.0 - &rhs.0);
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
        let reached = Target::from(hash);
        return &reached < self;
    }

    pub fn get_depth(&self) -> u8 {
        // Compute: 240 - ceil(log2(self))

        // Find first non-zero byte.
        let len = self.0.len();
        let mut first_byte = 0;
        for i in 0..len {
            if self.0[i] > 0 {
                first_byte = i;
                break;
            }
        }

        // Find last non-zero byte.
        let mut last_byte = 0;
        for i in 0..len - first_byte {
            let idx = len - i - 1;
            if self.0[idx] > 0 {
                last_byte = idx;
                break;
            }
        }

        let leading_zeros = self.0[first_byte].leading_zeros();
        let mut exp = 8 - leading_zeros + (len - first_byte - 1) as u32 * 8;

        if first_byte == last_byte && self.0[first_byte].trailing_zeros() + leading_zeros == 7 {
            exp -= 1;
        }

        return 240 - exp as u8;
    }
}
