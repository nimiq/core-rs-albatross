use serde::{de::Error, Deserialize, Deserializer, Serialize, Serializer};

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
        let s: &'de str = Deserialize::deserialize(deserializer)?;
        s.parse().map_err(Error::custom)
    }
}
