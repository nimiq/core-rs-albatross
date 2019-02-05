use beserial::{Serialize, SerializingError, Deserialize, ReadBytesExt};
use std::ops::{Add, Sub};
use std::io;

#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Eq, Ord, Serialize)]
pub struct Coin(u64);

impl Coin {
    pub const ZERO: Coin = Coin(0u64);

    // JavaScript's Number.MAX_SAFE_INTEGER: 2^53 - 1
    pub const MAX_SAFE_VALUE: u64 = 9007199254740991u64;
}

impl From<u64> for Coin {
    fn from(value: u64) -> Self { Coin(value) }
}

impl From<Coin> for u64 {
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
