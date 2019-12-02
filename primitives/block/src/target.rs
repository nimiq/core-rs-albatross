use std::fmt;
use std::ops::{Add, AddAssign, Div, Sub, SubAssign};

use num_bigint::BigUint;

use beserial::{Deserialize, DeserializeWithLength, ReadBytesExt, Serialize, SerializeWithLength, SerializingError, WriteBytesExt};
use fixed_unsigned::types::FixedUnsigned10;
use hash::Argon2dHash;
use macros::create_typed_array;
use primitives::policy;

/// Compact Target (represented internally as `u32`)
#[derive(Default, Clone, Copy, PartialEq, PartialOrd, Eq, Ord, Serialize, Deserialize)]
pub struct TargetCompact(u32);

impl fmt::Debug for TargetCompact {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "TargetCompact{{{:04x}: shift={}, value={:06x}}}", self.0, self.0 >> 24, self.0 & 0x00FF_FFFF)
    }
}

// Target as 32 byte array
create_typed_array!(Target, u8, 32);

/// Difficulty as FixedUnsigned with 10 decimal places
#[derive(Default, Clone, PartialEq, PartialOrd, Eq, Ord, Debug)]
pub struct Difficulty(FixedUnsigned10);

/// Convert `TargetCompact` to `u32`
impl From<TargetCompact> for u32 {
    fn from(t: TargetCompact) -> Self { t.0 }
}

/// Convert `u32` to `TargetCompact`
impl From<u32> for TargetCompact {
    fn from(u: u32) -> Self { TargetCompact(u) }
}

/// Convert `TargetCompact` to `Target`
impl From<TargetCompact> for Target {
    fn from(target_compact: TargetCompact) -> Self {
        let mut val = [0u8; 32];
        let shift_bytes = (target_compact.0 >> 24).saturating_sub(3) as usize;
        val[32 - shift_bytes - 1] = (target_compact.0 & 0xff) as u8;
        val[32 - shift_bytes - 2] = ((target_compact.0 >> 8) & 0xff) as u8;
        val[32 - shift_bytes - 3] = ((target_compact.0 >> 16) & 0xff) as u8;
        Target(val)
    }
}

/// Convert `Target` to `TargetCompact`
///
/// NOTE: This just delegates to `From<&Target> for TargetCompact`
impl From<Target> for TargetCompact {
    fn from(target: Target) -> Self {
        TargetCompact::from(&target)
    }
}

/// Convert `&Target` to `TargetCompact`
impl<'a> From<&'a Target> for TargetCompact {
    fn from(target: &'a Target) -> Self {
        // Determine where the first byte in target is.
        let mut first_byte = 0;
        for i in 0..target.0.len() {
            if target.0[i] > 0 {
                first_byte = i;
                break;
            }
        }

        // The significant is signed and therefore can't be > 0x80. If it is, we take the byte before it as first byte
        if target.0[first_byte] >= 0x80 && first_byte <= 29 {
            first_byte -= 1;
        }

        let shift_bytes = 32 - first_byte;
        let start_byte = first_byte.min(29);
        TargetCompact(((shift_bytes as u32) << 24)
            | (u32::from(target.0[start_byte]) << 16)
            | (u32::from(target.0[start_byte + 1]) << 8)
            | u32::from(target.0[start_byte + 2]))
    }
}

/// Convert `Argon2dHash` to `Target`
impl<'a> From<&'a Argon2dHash> for Target {
    fn from(hash: &'a Argon2dHash) -> Self {
        Target::from(hash.as_bytes())
    }
}

/// Convert `BigUint` to `Target`
impl From<BigUint> for Target {
    fn from(num: BigUint) -> Self {
        // Convert `BigUint` to big-endian byte represetation, this must have length <= 32
        let bytes = num.to_bytes_be();
        let byte_len = bytes.len();
        assert!(byte_len <= 32, "Cannot convert BigDecimal to Target - out of bounds");

        // Fill up front of array (most-siginificant bytes) with zeros
        let mut target_data = [0u8; 32];
        for i in 0..byte_len {
            target_data[32 - byte_len + i] = bytes[i];
        }

        Target(target_data)
    }
}

/// Convert `Target` to `BigUint`
impl From<Target> for BigUint {
    fn from(target: Target) -> BigUint {
        let target_data: [u8; 32] = target.into();
        BigUint::from_bytes_be(&target_data)
    }
}

/// Convert `Target` to `FixedUnsigned10`
impl From<Target> for FixedUnsigned10 {
    fn from(target: Target) -> FixedUnsigned10 {
        FixedUnsigned10::from(BigUint::from(target))
    }
}

/// Convert `TargetCompact` to `Difficulty`
impl From<TargetCompact> for Difficulty {
    fn from(target_compact: TargetCompact) -> Self {
        Target::from(target_compact).into()
    }
}

/// Convert `Target` to `Difficulty`
impl From<Target> for Difficulty {
    fn from(target: Target) -> Self {
        Difficulty(&*policy::BLOCK_TARGET_MAX / &FixedUnsigned10::from(target))
    }
}

/// Convert `FixedUnsigned10` to `Target`
impl From<FixedUnsigned10> for Target {
    fn from(fixed: FixedUnsigned10) -> Target {
        Target::from(fixed.into_biguint())
    }
}

/// XXX For testing only
impl From<Difficulty> for TargetCompact {
    fn from(difficulty: Difficulty) -> Self {
        Target::from(difficulty).into()
    }
}

/// XXX For testing only
impl From<Difficulty> for Target {
    fn from(difficulty: Difficulty) -> Self {
        Target::from(&*policy::BLOCK_TARGET_MAX / &FixedUnsigned10::from(difficulty))
    }
}

/// Converts a `i32` to a `Difficulty`
///
/// NOTE: This is just to please the old test gods, because the tests were implemented with
///       positive `i32`s.
///
/// XXX For testing only
impl From<i32> for Difficulty {
    fn from(x: i32) -> Difficulty {
        warn!("Conversion from `i32` to `Difficulty`. This will panic on negative numbers! Do not use!");
        if x < 0 {
            panic!("Can't convert a `i32` into a `Difficulty`!");
        }
        Difficulty(FixedUnsigned10::from(x as u32))
    }
}

/// Convert a `FixedUnsigned10` (10 decimal places) to `Difficulty`
impl From<FixedUnsigned10> for Difficulty {
    fn from(fixed: FixedUnsigned10) -> Self {
        Difficulty(fixed)
    }
}

impl From<Difficulty> for FixedUnsigned10 {
    fn from(difficulty: Difficulty) -> Self { difficulty.0 }
}

/// XXX Only for debugging - or is it?
impl From<u64> for Difficulty {
    fn from(x: u64) -> Difficulty {
        Difficulty(FixedUnsigned10::from(x))
    }
}

/// Convert `u32` to `Difficulty`
impl From<u32> for Difficulty {
    fn from(x: u32) -> Difficulty {
        Difficulty(FixedUnsigned10::from(x))
    }
}

/// Add a `u32` onto a `Difficulty`
///
/// This is used in `Blockchain::get_next_target` to compute the `delta_total_difficulty`:
impl AddAssign<u32> for Difficulty {
    fn add_assign(&mut self, rhs: u32) {
        *self += Difficulty(FixedUnsigned10::from(rhs));
    }
}

/// Divide `Difficulty` by `u32`
///
/// This is used in `Blockchain::get_next_target`
///
/// NOTE: I chose to implement exactly those traits with these types to abstract away how
///       `Difficulty` works from the `Blockchain`
impl Div<u32> for Difficulty {
    type Output = Difficulty;

    fn div(self, rhs: u32) -> <Self as Div<u32>>::Output {
        Difficulty(self.0 / FixedUnsigned10::from(rhs))
    }
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
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        SerializeWithLength::serialize::<u8, W>(&self.0.to_bytes_be(), writer)
    }

    fn serialized_size(&self) -> usize {
        self.0.bytes() + 1 // 1 byte for length
    }
}

impl Deserialize for Difficulty {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
        let bytes: Vec<u8> = DeserializeWithLength::deserialize::<u8, R>(reader)?;
        Ok(Difficulty(FixedUnsigned10::from_bytes_be(bytes.as_slice())))
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
        reached < *self
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

        240u8.saturating_sub(exp as u8)
    }
}
