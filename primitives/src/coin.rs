use std::ops::{Add, Sub};
use std::io;
use std::fmt;
use std::error::Error;
use std::str::FromStr;

use beserial::{Serialize, SerializingError, Deserialize, ReadBytesExt, WriteBytesExt};


#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Eq, Ord)]
pub struct Coin(u64);

impl Coin {
    pub const ZERO: Coin = Coin(0u64);

    // How many Lunas fit in once Coin
    pub const LUNAS_PER_COIN: u64 = 100000u64;
    pub const FRAC_DIGITS: u32 = 5u32;

    // JavaScript's Number.MAX_SAFE_INTEGER: 2^53 - 1
    pub const MAX_SAFE_VALUE: u64 = 9007199254740991u64;
}

impl From<u64> for Coin {
    fn from(value: u64) -> Self { Coin(value) }
}

impl From<Coin> for u64 {
    // TODO: Should this panic with an unsafe value? or should it return an Option?
    // NOTE: nightly rust has TryFrom
    fn from(coin: Coin) -> Self { coin.0 }
}

impl Add<Coin> for Coin {
    type Output = Coin;

    fn add(self, rhs: Coin) -> Coin {
        return Coin(self.0 + rhs.0);
    }
}

impl Sub<Coin> for Coin {
    type Output = Coin;

    fn sub(self, rhs: Coin) -> Coin {
        return Coin(self.0 - rhs.0);
    }
}

impl fmt::Display for Coin {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        // NOTE: The format string has 5 decimal places hard-coded
        let frac_part = self.0 % Coin::LUNAS_PER_COIN;
        if frac_part == 0 {
            write!(f, "{}", self.0 / Coin::LUNAS_PER_COIN)
        }
        else {
            write!(f, "{}.{:05}", self.0 / Coin::LUNAS_PER_COIN, self.0 % Coin::LUNAS_PER_COIN)
        }
    }
}

impl Deserialize for Coin {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
        let value: u64 = Deserialize::deserialize(reader)?;

        // Check that the value does not exceed Javascript's Number.MAX_SAFE_INTEGER.
        return match value <= Coin::MAX_SAFE_VALUE {
            true => Ok(Coin(value)),
            false => Err(io::Error::new(io::ErrorKind::InvalidData, "Coin value out of bounds").into())
        };
    }
}

impl Serialize for Coin {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        if self.0 <= Coin::MAX_SAFE_VALUE {
            Ok(Serialize::serialize(&self.0, writer)?)
        } else {
            error!("Coin value out of bounds");
            Err(SerializingError::Overflow)
        }
    }

    fn serialized_size(&self) -> usize {
        Serialize::serialized_size(&self.0)
    }
}


// TODO: This should also check for MAX_SAFE_VALUE
impl Coin {
    pub fn checked_add(self, rhs: Coin) -> Option<Coin> {
        self.0.checked_add(rhs.0).map(|v| Coin(v))
    }

    pub fn checked_sub(self, rhs: Coin) -> Option<Coin> {
        self.0.checked_sub(rhs.0).map(|v| Coin(v))
    }

    pub fn checked_factor(self, times: u64) -> Option<Coin> {
        self.0.checked_mul(times).map(|v| Coin(v))
    }
}


#[derive(Debug)]
pub enum CoinParseError {
    InvalidString,
    TooManyFractionalDigits,
    Overflow
}

impl fmt::Display for CoinParseError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.description())
    }
}

impl Error for CoinParseError {
    fn description(&self) -> &str {
        match self {
            CoinParseError::InvalidString => "Invalid String",
            CoinParseError::TooManyFractionalDigits => "Too may fractional digits",
            CoinParseError::Overflow => "Overflow"
        }
    }

    fn cause(&self) -> Option<&Error> {
        None
    }
}


impl FromStr for Coin {
    type Err = CoinParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // NOTE: I would like to use a RegEx here, but this needs a crate - Janosch
        let split: Vec<&str> = s.split('.').into_iter().collect();

        // check that string is either 1 part or 2 parts seperated by a `.`
        if split.len() < 1 || split.len() > 2 {
            return Err(CoinParseError::InvalidString);
        }
        // try to parse integer part
        // TODO: Do we accept int_part == "" as being 0?
        let int_part = split[0].parse::<u64>().map_err(|_| CoinParseError::InvalidString)?;
        // try to parse fractional part, if available
        // TODO: Do we accept frac_part == "" as being 0?
        let frac_part = if split.len() == 2 {
            // check that number of digits doesn't overflow MAX_VALUE of u8
            if split[1].len() > (std::u8::MAX as usize) {
                return Err(CoinParseError::TooManyFractionalDigits);
            }
            // check how many digits we have in the fraction and multiply by 10^(DIGITS - n)
            // This cast is safe, since we checked that it doesn't overflow an u8
            let frac_len = split[1].len() as u32;
            if frac_len > Coin::FRAC_DIGITS {
                Err(CoinParseError::TooManyFractionalDigits)
            }
            else {
                Ok(10u64.pow(Coin::FRAC_DIGITS - frac_len))
            }? * split[1].parse::<u64>().map_err(|_| CoinParseError::InvalidString)?
        }
        else { 0u64 };
        // check that digits out of our precision (5 digits) are all 0
        if frac_part / Coin::LUNAS_PER_COIN > 0 {
            return Err(CoinParseError::TooManyFractionalDigits);
        }

        // multiply int_part with LUNAS_PER_COIN and add frac_part
        let value = int_part
            .checked_mul(Coin::LUNAS_PER_COIN).ok_or(CoinParseError::Overflow)?
            .checked_add(frac_part).ok_or(CoinParseError::Overflow)?;

        // Check if value is Javascript-safe
        if value > Coin::MAX_SAFE_VALUE {
            Err(CoinParseError::Overflow)
        }
        else {
            Ok(Coin::from(value))
        }
    }
}
