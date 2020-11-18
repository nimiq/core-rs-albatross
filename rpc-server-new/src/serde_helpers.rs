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
        Serialize::serialize(&u8::from(*account_type), serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<AccountType, D::Error>
    where
        D: Deserializer<'de>,
    {
        let n: u8 = Deserialize::deserialize(deserializer)?;
        AccountType::try_from(n).map_err(D::Error::custom)
    }
}

pub mod address_hex {
    use serde::{
        de::{Deserialize, Deserializer, Error},
        ser::{Serialize, Serializer},
    };

    use nimiq_keys::Address;

    pub fn serialize<S>(address: &Address, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        Serialize::serialize(&address.to_hex(), serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Address, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s: &'de str = Deserialize::deserialize(deserializer)?;
        s.parse().map_err(D::Error::custom)
    }
}

pub mod address_friendly {
    use serde::{
        de::{Deserialize, Deserializer, Error},
        ser::{Serialize, Serializer},
    };

    use nimiq_keys::Address;

    pub fn serialize<S>(address: &Address, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        Serialize::serialize(&address.to_user_friendly_address(), serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Address, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s: &'de str = Deserialize::deserialize(deserializer)?;
        Address::from_user_friendly_address(s).map_err(D::Error::custom)
    }
}

pub mod hex {
    // TODO: Make generic over `ToHex` and `FromHex`. Or use `serde_hex`

    use serde::{
        de::{Deserialize, Deserializer, Error},
        ser::{Serialize, Serializer},
    };

    pub fn serialize<S>(x: &Vec<u8>, serializer: S) -> Result<S::Ok, S::Error>
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
