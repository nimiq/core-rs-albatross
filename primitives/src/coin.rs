use std::convert::TryFrom;
use std::fmt;
use std::io;
use std::iter::Sum;
use std::ops::{Add, AddAssign, Div, Rem, Sub, SubAssign};
use std::str::FromStr;

use lazy_static::lazy_static;
use num_traits::identities::Zero;
use regex::Regex;
use thiserror::Error;

use beserial::{Deserialize, ReadBytesExt, Serialize, SerializingError, WriteBytesExt};

#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Eq, Ord, Default)]
#[cfg_attr(feature = "serde-derive", derive(serde::Serialize, serde::Deserialize), serde(transparent))]
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
    // a whole crate to check if a coin value is zero.
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

#[derive(Debug, Error, PartialEq, Eq)]
#[error("Can't convert u64 to Coin value: {0}")]
pub struct CoinConvertError(u64);

impl TryFrom<u64> for Coin {
    type Error = CoinConvertError;

    #[inline]
    fn try_from(val: u64) -> Result<Self, Self::Error> {
        if val <= Coin::MAX_SAFE_VALUE {
            Ok(Coin(val))
        } else {
            Err(CoinConvertError(val))
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

impl Sum for Coin {
    fn sum<I: Iterator<Item = Coin>>(iter: I) -> Self {
        iter.fold(Coin::ZERO, Add::add)
    }
}

impl fmt::Display for Coin {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        // NOTE: The format string has 5 decimal places hard-coded
        let frac_part = self.0 % Coin::LUNAS_PER_COIN;
        if frac_part == 0 {
            write!(f, "{}", self.0 / Coin::LUNAS_PER_COIN)
        } else {
            write!(f, "{}.{:05}", self.0 / Coin::LUNAS_PER_COIN, self.0 % Coin::LUNAS_PER_COIN)
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

#[derive(Debug, Error)]
#[error("Can't parse Coin value: '{0}'")]
pub struct CoinParseError(String);

impl Eq for CoinParseError {}

impl PartialEq for CoinParseError {
    fn eq(&self, _other: &CoinParseError) -> bool {
        true
    }
}

lazy_static! {
    /// This is an example for using doc comment attributes
    static ref COIN_PARSE_REGEX: Regex = {
        let r = r"^(?P<int_part>\d+)(.(?P<frac_part>\d{1,5})0*)?$";
        Regex::new(r)
            .unwrap_or_else(|e| panic!("Failed to compile regex: {}: {}", r, e))
    };
}

impl FromStr for Coin {
    type Err = CoinParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let e = || CoinParseError(s.to_owned());

        let captures = COIN_PARSE_REGEX.captures(s).ok_or_else(e)?;

        let int_part = captures.name("int_part").ok_or_else(e)?.as_str().parse::<u64>().map_err(|_| e())?;

        let frac_part = if let Some(frac_part_capture) = captures.name("frac_part") {
            let frac_part_str = frac_part_capture.as_str();
            let mut frac_part = frac_part_str.parse::<u64>().map_err(|_| e())?;
            frac_part *= 10_u64.pow(Coin::FRAC_DIGITS - frac_part_str.len() as u32);
            frac_part
        } else {
            0
        };

        let coin = Coin::try_from(int_part * Coin::LUNAS_PER_COIN + frac_part).map_err(|_| e())?;

        Ok(coin)
    }
}
