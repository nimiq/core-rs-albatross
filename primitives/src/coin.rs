use std::convert::TryFrom;
use std::fmt;
use std::io;
use std::ops::{Add, AddAssign, Div, Rem, Sub, SubAssign};
use std::str::FromStr;

use failure::Fail;
use num_traits::identities::Zero;

use beserial::{Deserialize, ReadBytesExt, Serialize, SerializingError, WriteBytesExt};

#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Eq, Ord, Default)]
pub struct Coin(u64);

impl Coin {
    pub const ZERO: Coin = Coin(0u64);

    // How many Lunas fit in one Coin
    pub const LUNAS_PER_COIN: u64 = 100_000u64;
    pub const FRAC_DIGITS: u32 = 5u32;

    // JavaScript's Number.MAX_SAFE_INTEGER: 2^53 - 1
    pub const MAX_SAFE_VALUE: u64 = 9_007_199_254_740_991u64;

    #[inline]
    pub fn from_u64_unchecked(val: u64) -> Coin {
        Coin(val)
    }

    // NOTE: We implement a trait that does this, but we don't always want to have to import
    // a whole trait to check if a coin value is zero.
    pub fn is_zero(&self) -> bool {
        self.0 == 0
    }

    #[inline]
    pub fn checked_add(self, rhs: Coin) -> Option<Coin> {
        match self.0.checked_add(rhs.0) {
            Some(val) => Coin::try_from(val).ok(),
            None => None,
        }
    }

    #[inline]
    pub fn checked_sub(self, rhs: Coin) -> Option<Coin> {
        match self.0.checked_sub(rhs.0) {
            Some(val) => Coin::try_from(val).ok(),
            None => None,
        }
    }

    #[inline]
    pub fn checked_mul(self, times: u64) -> Option<Coin> {
        match self.0.checked_mul(times) {
            Some(val) => Coin::try_from(val).ok(),
            None => None,
        }
    }
}

impl From<Coin> for u64 {
    #[inline]
    fn from(coin: Coin) -> Self {
        coin.0
    }
}

impl TryFrom<u64> for Coin {
    type Error = CoinParseError;

    #[inline]
    fn try_from(val: u64) -> Result<Self, Self::Error> {
        if val <= Coin::MAX_SAFE_VALUE {
            Ok(Coin(val))
        } else {
            Err(CoinParseError::Overflow)
        }
    }
}

impl Add<Coin> for Coin {
    type Output = Coin;

    #[inline]
    fn add(self, rhs: Coin) -> Coin {
        Coin(self.0 + rhs.0)
    }
}

impl AddAssign<Coin> for Coin {
    #[inline]
    fn add_assign(&mut self, rhs: Coin) {
        self.0 += rhs.0;
    }
}

impl Sub<Coin> for Coin {
    type Output = Coin;

    #[inline]
    fn sub(self, rhs: Coin) -> Coin {
        Coin(self.0 - rhs.0)
    }
}

impl SubAssign<Coin> for Coin {
    #[inline]
    fn sub_assign(&mut self, rhs: Coin) {
        self.0 -= rhs.0;
    }
}

impl Div<u64> for Coin {
    type Output = Coin;

    #[inline]
    fn div(self, rhs: u64) -> Self {
        Coin(self.0 / rhs)
    }
}

impl Rem<u64> for Coin {
    type Output = Coin;

    #[inline]
    fn rem(self, rhs: u64) -> Self {
        Coin(self.0 % rhs)
    }
}

impl Zero for Coin {
    fn zero() -> Self {
        Self::ZERO
    }

    fn is_zero(&self) -> bool {
        self.0 == 0
    }
}

impl fmt::Display for Coin {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        // NOTE: The format string has 5 decimal places hard-coded
        let frac_part = self.0 % Coin::LUNAS_PER_COIN;
        if frac_part == 0 {
            write!(f, "{}", self.0 / Coin::LUNAS_PER_COIN)
        } else {
            write!(
                f,
                "{}.{:05}",
                self.0 / Coin::LUNAS_PER_COIN,
                self.0 % Coin::LUNAS_PER_COIN
            )
        }
    }
}

impl Deserialize for Coin {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
        let value: u64 = Deserialize::deserialize(reader)?;

        // Check that the value does not exceed Javascript's Number.MAX_SAFE_INTEGER.
        if value <= Coin::MAX_SAFE_VALUE {
            Ok(Coin(value))
        } else {
            Err(io::Error::new(io::ErrorKind::InvalidData, "Coin value out of bounds").into())
        }
    }
}

impl Serialize for Coin {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        if self.0 <= Coin::MAX_SAFE_VALUE {
            Ok(Serialize::serialize(&self.0, writer)?)
        } else {
            Err(SerializingError::Overflow)
        }
    }

    fn serialized_size(&self) -> usize {
        Serialize::serialized_size(&self.0)
    }
}

#[derive(Debug, Fail)]
pub enum CoinParseError {
    #[fail(display = "Invalid string")]
    InvalidString,
    #[fail(display = "Too many fractional digits")]
    TooManyFractionalDigits,
    #[fail(display = "Overflow or unsafe value")]
    Overflow,
}

impl FromStr for Coin {
    type Err = CoinParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // NOTE: I would like to use a RegEx here, but this needs a crate - Janosch
        let split: Vec<&str> = s.split('.').collect();

        // Check that string is either 1 part or 2 parts separated by a `.`
        if split.len() != 1 && split.len() != 2 {
            return Err(CoinParseError::InvalidString);
        }
        // Try to parse integer part
        // TODO: Do we accept int_part == "" as being 0?
        let int_part = split[0]
            .parse::<u64>()
            .map_err(|_| CoinParseError::InvalidString)?;

        // Try to parse fractional part, if available
        // TODO: Do we accept frac_part == "" as being 0?
        let frac_part = if split.len() == 2 {
            // Check that number of digits doesn't overflow MAX_VALUE of u8
            if split[1].len() > (std::u8::MAX as usize) {
                return Err(CoinParseError::TooManyFractionalDigits);
            }
            // Check how many digits we have in the fraction and multiply by 10^(DIGITS - n)
            // This cast is safe, since we checked that it doesn't overflow an u8
            let frac_len = split[1].len() as u32;
            if frac_len > Coin::FRAC_DIGITS {
                Err(CoinParseError::TooManyFractionalDigits)
            } else {
                Ok(10u64.pow(Coin::FRAC_DIGITS - frac_len))
            }? * split[1]
                .parse::<u64>()
                .map_err(|_| CoinParseError::InvalidString)?
        } else {
            0u64
        };

        // Check that digits out of our precision (5 digits) are all 0
        if frac_part / Coin::LUNAS_PER_COIN > 0 {
            return Err(CoinParseError::TooManyFractionalDigits);
        }

        // Multiply int_part with LUNAS_PER_COIN and add frac_part
        let value = int_part
            .checked_mul(Coin::LUNAS_PER_COIN)
            .ok_or(CoinParseError::Overflow)?
            .checked_add(frac_part)
            .ok_or(CoinParseError::Overflow)?;

        Ok(Coin::try_from(value)?)
    }
}
