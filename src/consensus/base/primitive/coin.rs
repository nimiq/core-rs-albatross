use beserial::{Serialize, Deserialize, ReadBytesExt};
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
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> io::Result<Self> {
        let value: u64 = Deserialize::deserialize(reader)?;

        // Check that the value does not exceed Javascript's Number.MAX_SAFE_INTEGER.
        return match value <= Coin::MAX_SAFE_VALUE {
            true => Ok(Coin(value)),
            false => Err(io::Error::new(io::ErrorKind::InvalidData, "Coin value out of bounds"))
        };
    }
}

impl Coin {
    pub fn checked_add(self, rhs: Coin) -> Option<Coin> {
        let value: u64 = match self.0.checked_add(rhs.0) {
            Some(value) => value,
            None => return None
        };
        return match value <= Coin::MAX_SAFE_VALUE {
            true => Some(Coin(value)),
            false => None
        };
    }

    pub fn checked_sub(self, rhs: Coin) -> Option<Coin> {
        return match self.0.checked_sub(rhs.0) {
            Some(value) => Some(Coin(value)),
            None => None
        };
    }

}
