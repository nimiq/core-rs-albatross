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
        hex::decode(String::deserialize(deserializer)?).map_err(Error::custom)
    }
}
