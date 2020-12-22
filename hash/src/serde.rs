use std::{
    borrow::Cow,
    fmt
};

use serde::{de::{Error, Visitor}, Deserialize, Deserializer, Serialize, Serializer};

use crate::Blake2bHash;


impl Serialize for Blake2bHash {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        Serialize::serialize(&self.to_hex(), serializer)
    }
}

impl<'de> Deserialize<'de> for Blake2bHash {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s: Cow<'de, str> = Deserialize::deserialize(deserializer)?;
        s.parse().map_err(Error::custom)
    }
}

/*
struct Blake2bHashVisitor;

impl Blake2bHashVisitor {
    fn block_hash<E>(&self, s: &str) -> Result<Blake2bHash, E>
        where E: Error,
    {
        s.parse().map_err(Error::custom)
    }
}

impl<'de> Visitor<'de> for Blake2bHashVisitor {
    type Value = Blake2bHash;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        write!(formatter, "a string")
    }

    fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
        where E: Error
    {
        self.block_hash(v)
    }

    fn visit_borrowed_str<E>(self, v: &'de str) -> Result<Self::Value, E>
        where E: Error
    {
        self.block_hash(v)
    }

    fn visit_string<E>(self, v: String) -> Result<Self::Value, E>
        where E: Error
    {
        self.block_hash(&v)
    }
}
*/
