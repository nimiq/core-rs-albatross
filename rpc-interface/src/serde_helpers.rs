pub mod account_type {
    use std::convert::TryFrom;

    use serde::{
        de::{Deserialize, Deserializer, Error},
        ser::{Serialize, Serializer},
    };

    use nimiq_primitives::account::AccountType;

    pub fn serialize<S>(account_type: &AccountType, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let ty = match account_type {
            AccountType::Basic => "basic",
            AccountType::Vesting => "vesting",
            AccountType::HTLC => "htlc",
            _ => unreachable!(),
        };
        Serialize::serialize(ty, serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<AccountType, D::Error>
    where
        D: Deserializer<'de>,
    {
        let ty: AccountType = match Deserialize::deserialize(deserializer)? {
            "basic" => AccountType::Basic,
            "vesting" => AccountType::Vesting,
            "htlc" => AccountType::HTLC,
            _ => unreachable!(),
        };
        AccountType::try_from(ty).map_err(D::Error::custom)
    }
}

pub mod hex {
    // TODO: Make generic over `ToHex` and `FromHex`. Or use `serde_hex`

    use serde::{
        de::{Deserialize, Deserializer, Error},
        ser::{Serialize, Serializer},
    };

    pub fn serialize<S>(x: &[u8], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        Serialize::serialize(&hex::encode(x), serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Vec<u8>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s: &'de str = Deserialize::deserialize(deserializer)?;
        hex::decode(s).map_err(Error::custom)
    }
}
