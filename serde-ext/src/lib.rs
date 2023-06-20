use std::fmt::Debug;

use serde::{
    de::{DeserializeOwned, Deserializer, Error},
    ser::Serializer,
    Deserialize, Serialize,
};
use serde_big_array::BigArray;

/// The Nimiq human readable array serialization helper trait
///
/// ```
/// # use serde::{Serialize, Deserialize};
/// # use nimiq_serde_ext::HexArray;
/// #[derive(Serialize, Deserialize)]
/// struct S {
///     #[serde(with = "HexArray")]
///     arr: [u8; 64],
/// }
/// ```
pub trait HexArray<'de>: Sized {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer;
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>;
}

impl<'de, const N: usize> HexArray<'de> for [u8; N] {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        if serializer.is_human_readable() {
            serializer.serialize_str(&hex::encode(self))
        } else {
            BigArray::serialize(self, serializer)
        }
    }

    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        if deserializer.is_human_readable() {
            let s: String = Deserialize::deserialize(deserializer)?;
            let mut out = [0u8; N];
            hex::decode_to_slice(s, &mut out)
                .map_err(|_| D::Error::custom("Couldn't decode hex string"))?;
            Ok(out)
        } else {
            BigArray::deserialize(deserializer)
        }
    }
}

/// The Nimiq wrapper for serializing std::ops::RangeFrom
#[derive(Clone, Debug)]
pub struct SerRangeFrom<T: Clone + Debug + Serialize + DeserializeOwned>(
    pub std::ops::RangeFrom<T>,
);

impl<T: Clone + Debug + Serialize + DeserializeOwned> serde::Serialize for SerRangeFrom<T> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        Serialize::serialize(&self.0.start, serializer)
    }
}

impl<'de, T: Clone + Debug + Serialize + DeserializeOwned> Deserialize<'de> for SerRangeFrom<T> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let start: T = Deserialize::deserialize(deserializer)?;
        Ok(Self(std::ops::RangeFrom { start }))
    }
}
