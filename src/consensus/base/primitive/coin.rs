use beserial::{Serialize, Deserialize, ReadBytesExt};
use std::ops::{Add, Sub};
use std::io;

#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Eq, Ord, Serialize)]
pub struct Coin(pub u64);

impl Coin {
    pub const ZERO: Coin = Coin(0u64);

    // JavaScript Number.MAX_SAFE_INTEGER: 2^53 - 1
    pub const MAX_SAFE_VALUE: u64 = 9007199254740991u64;
}

impl Add<Coin> for Coin {
    type Output = Option<Coin>;

    fn add(self, rhs: Coin) -> Option<Coin> {
        // TODO Should we check for Number.MAX_SAFE_INTEGER here as well?
        return match self.0.checked_add(rhs.0) {
            Some(value) => Some(Coin(value)),
            None => None
        }
    }
}

impl Sub<Coin> for Coin {
    type Output = Option<Coin>;

    fn sub(self, rhs: Coin) -> Option<Coin> {
        return match self.0.checked_sub(rhs.0) {
            Some(value) => Some(Coin(value)),
            None => None
        }
    }
}

impl Deserialize for Coin {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> io::Result<Self> {
        let value: u64 = Deserialize::deserialize(reader)?;

        // Check that the value does not exceed Javascript's Number.MAX_SAFE_INTEGER.
        return if value > Coin::MAX_SAFE_VALUE {
            Err(io::Error::new(io::ErrorKind::InvalidData, "Coin value out of bounds"))
        } else {
            Ok(Coin(value))
        }
    }
}
