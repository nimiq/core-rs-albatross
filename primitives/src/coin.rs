use std::{
    convert::TryFrom,
    fmt,
    iter::Sum,
    ops::{Add, AddAssign, Div, Rem, Sub, SubAssign},
    str::FromStr,
    sync::OnceLock,
};

use regex::Regex;
use thiserror::Error;

/// An amount of NIM.
///
/// Internally stored as a 64-bit integer of LUNAs.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Eq, Ord, Default, Hash)]
pub struct Coin(u64);

impl Coin {
    /// 0 NIM.
    pub const ZERO: Coin = Coin(0u64);
    /// The maximum value supported by the [`Coin`] type.
    pub const MAX: Coin = Coin(Self::MAX_SAFE_VALUE);

    /// How many Lunas fit in one Coin
    pub const LUNAS_PER_COIN: u64 = 100_000u64;
    /// The number of digits after the decimal point for NIM values, ten to the
    /// power of this is [`Coin::LUNAS_PER_COIN`].
    pub const FRAC_DIGITS: u32 = 5u32;

    /// JavaScript's Number.MAX_SAFE_INTEGER: 2**53 - 1
    ///
    /// Bounding NIM values to this number ensures that they survive
    /// `f64`-`i64` conversions without precision loss.
    pub const MAX_SAFE_VALUE: u64 = 9_007_199_254_740_991u64;

    /// Creates a `Coin` object from an amount of LUNAs.
    ///
    /// # Panics
    ///
    /// Panics if the amount of LUNAs is greater than [`Coin::MAX_SAFE_VALUE`].
    #[inline]
    pub fn from_u64_unchecked(val: u64) -> Coin {
        val.try_into().expect("Coin::from_u64_unchecked")
    }

    /// Check if the NIM value is exactly 0.
    #[inline]
    pub fn is_zero(&self) -> bool {
        self.0 == 0
    }

    /// Checked addition. Computes `self + rhs`, returning `None` if the result
    /// is not representable.
    pub fn checked_add(self, rhs: Coin) -> Option<Coin> {
        self.0
            .checked_add(rhs.0)
            .and_then(|val| Coin::try_from(val).ok())
    }

    /// Checked subtraction. Computes `self - rhs`, returning `None` if the
    /// result is not representable.
    pub fn checked_sub(self, rhs: Coin) -> Option<Coin> {
        self.0
            .checked_sub(rhs.0)
            .and_then(|val| Coin::try_from(val).ok())
    }

    /// Checked multiplication. Computes `self * rhs`, returning `None` if the
    /// result is not representable.
    pub fn checked_mul(self, rhs: u64) -> Option<Coin> {
        self.0
            .checked_mul(rhs)
            .and_then(|val| Coin::try_from(val).ok())
    }

    /// Checked subtraction. Computes `self - rhs`, returning
    /// [`CoinUnderflowError`] if the result is not representable.
    pub fn safe_sub(self, rhs: Coin) -> Result<Coin, CoinUnderflowError> {
        self.checked_sub(rhs)
            .ok_or(CoinUnderflowError { lhs: self, rhs })
    }

    /// Checked subtraction. Executes `self -= rhs`, returning
    /// [`CoinUnderflowError`] if the result is not representable.
    pub fn safe_sub_assign(&mut self, rhs: Coin) -> Result<(), CoinUnderflowError> {
        *self = self.safe_sub(rhs)?;
        Ok(())
    }

    /// Saturating addition. Computes `self + rhs`, returning `Coin::MAX` if
    /// the result would not be representable otherwise.
    pub fn saturating_add(self, rhs: Coin) -> Coin {
        self.checked_add(rhs).unwrap_or(Coin::MAX)
    }

    /// Saturating subtraction. Computes `self - rhs`, returning `Coin::ZERO`
    /// if the result would not be representable otherwise.
    pub fn saturating_sub(self, rhs: Coin) -> Coin {
        Coin(self.0.saturating_sub(rhs.0))
    }
}

/// Convert to an amount of LUNAs.
impl From<Coin> for u64 {
    #[inline]
    fn from(coin: Coin) -> Self {
        coin.0
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
#[error("Underflow: {lhs} - {rhs}")]
pub struct CoinUnderflowError {
    pub lhs: Coin,
    pub rhs: Coin,
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
    fn add(self, rhs: Coin) -> Self {
        self.checked_add(rhs).expect("Overflow during add()")
    }
}

impl AddAssign<Coin> for Coin {
    #[inline]
    fn add_assign(&mut self, rhs: Coin) {
        *self = self.checked_add(rhs).expect("Overflow during add_assign()")
    }
}

impl Sub<Coin> for Coin {
    type Output = Coin;

    #[inline]
    fn sub(self, rhs: Coin) -> Self {
        self.checked_sub(rhs).expect("Underflow during sub()")
    }
}

impl SubAssign<Coin> for Coin {
    #[inline]
    fn sub_assign(&mut self, rhs: Coin) {
        *self = self
            .checked_sub(rhs)
            .expect("Underflow during sub_assign()")
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
            write!(
                f,
                "{}.{:05}",
                self.0 / Coin::LUNAS_PER_COIN,
                self.0 % Coin::LUNAS_PER_COIN
            )
        }
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

impl FromStr for Coin {
    type Err = CoinParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let e = || CoinParseError(s.to_owned());

        static REGEX: OnceLock<Regex> = OnceLock::new();
        let captures = REGEX
            .get_or_init(|| {
                let r = r"^(?P<int_part>\d+)(.(?P<frac_part>\d{1,5})0*)?$";
                Regex::new(r).unwrap_or_else(|e| panic!("Failed to compile regex: {r}: {e}"))
            })
            .captures(s)
            .ok_or_else(e)?;

        let int_part = captures
            .name("int_part")
            .ok_or_else(e)?
            .as_str()
            .parse::<u64>()
            .map_err(|_| e())?;

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

#[cfg(feature = "serde-derive")]
mod serialization {
    use nimiq_serde::SerializedSize;
    use serde::{
        de::{Error as DeError, Unexpected},
        ser::Error as SerError,
        Deserialize, Deserializer, Serialize, Serializer,
    };

    use super::*;

    impl SerializedSize for Coin {
        const SIZE: usize = 8;
    }

    impl Serialize for Coin {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            if self.0 <= Coin::MAX_SAFE_VALUE {
                if serializer.is_human_readable() {
                    self.0.serialize(serializer)
                } else {
                    nimiq_serde::fixint::be::serialize(&self.0, serializer)
                }
            } else {
                Err(S::Error::custom("Overflow detected for a Coin value"))
            }
        }
    }

    impl<'de> Deserialize<'de> for Coin {
        fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where
            D: Deserializer<'de>,
        {
            let value: u64 = if deserializer.is_human_readable() {
                Deserialize::deserialize(deserializer)?
            } else {
                nimiq_serde::fixint::be::deserialize(deserializer)?
            };
            Coin::try_from(value).map_err(|_| {
                D::Error::invalid_value(
                    Unexpected::Unsigned(value),
                    &"An u64 below the Coin maximum value",
                )
            })
        }
    }

    // Test must live here as we cannot create an out-of-range `Coin` from the
    // outside.
    #[test]
    fn test_serialize_out_of_bounds() {
        let mut vec = Vec::with_capacity(8);
        let res = nimiq_serde::Serialize::serialize_to_writer(&Coin(9007199254740992), &mut vec);
        match res {
            Ok(_) => panic!("Didn't fail"),
            Err(err) => assert_eq!(err.kind(), std::io::ErrorKind::Other),
        }
    }
}
