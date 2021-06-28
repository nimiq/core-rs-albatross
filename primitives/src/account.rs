use std::convert::{TryFrom, TryInto};

use strum_macros::Display;
use thiserror::Error;

use nimiq_macros::{add_hex_io_fns_typed_arr, create_typed_array};

use beserial::{Deserialize, Serialize};

#[derive(Clone, Copy, PartialEq, PartialOrd, Eq, Ord, Debug, Serialize, Deserialize, Display)]
#[repr(u8)]
#[cfg_attr(
    feature = "serde-derive",
    derive(serde::Serialize, serde::Deserialize),
    serde(try_from = "u8", into = "u8")
)]
pub enum AccountType {
    Basic = 0,
    Vesting = 1,
    HTLC = 2,
    Staking = 3,
}

impl AccountType {
    #[deprecated]
    /// Deprecated: Use `TryFrom`
    pub fn from_int(x: u8) -> Option<AccountType> {
        x.try_into().ok()
    }
}

#[derive(Debug, Error)]
#[error("Can't convert {0} to AccountType.")]
pub struct Error(u8);

impl TryFrom<u8> for AccountType {
    type Error = Error;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(AccountType::Basic),
            1 => Ok(AccountType::Vesting),
            2 => Ok(AccountType::HTLC),
            3 => Ok(AccountType::Staking),
            _ => Err(Error(value)),
        }
    }
}

impl From<AccountType> for u8 {
    fn from(ty: AccountType) -> Self {
        match ty {
            AccountType::Basic => 0,
            AccountType::Vesting => 1,
            AccountType::HTLC => 2,
            AccountType::Staking => 3,
            AccountType::Reward => 4,
        }
    }
}

create_typed_array!(ValidatorId, u8, 20);
add_hex_io_fns_typed_arr!(ValidatorId, ValidatorId::SIZE);

#[cfg(feature = "serde-derive")]
mod serde_derive {
    use std::borrow::Cow;

    use serde::{
        de::{Deserialize, Deserializer, Error},
        ser::{Serialize, Serializer},
    };

    use super::ValidatorId;

    impl Serialize for ValidatorId {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            serializer.serialize_str(&self.to_hex())
        }
    }

    impl<'de> Deserialize<'de> for ValidatorId {
        fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where
            D: Deserializer<'de>,
        {
            let data: Cow<'de, str> = Deserialize::deserialize(deserializer)?;
            data.parse().map_err(Error::custom)
        }
    }
}
