use std::convert::TryFrom;
use std::fmt::Display;
use std::str::FromStr;
use std::collections::HashMap;

use serde::{Deserialize, Deserializer};
use serde::de::Error;
use primitives::coin::Coin;


pub(crate) fn deserialize_coin<'de, D>(deserializer: D) -> Result<Coin, D::Error> where D: Deserializer<'de> {
    let value = u64::deserialize(deserializer)?;
    Coin::try_from(value).map_err(Error::custom)
}

pub(crate) fn deserialize_string<'de, D, T>(deserializer: D) -> Result<T, D::Error>
    where D: Deserializer<'de>,
          T: FromStr,
          T::Err: Display {
    let value = String::deserialize(deserializer)?;
    T::from_str(&value).map_err(Error::custom)
}

// NOTE: This is currently unused, but might be used in future.
#[allow(dead_code)]
pub(crate) fn deserialize_string_vec<'de, D, T>(deserializer: D) -> Result<Vec<T>, D::Error>
    where D: Deserializer<'de>,
          T: FromStr,
          T::Err: Display {
    let values = Vec::<String>::deserialize(deserializer)?;
    values.iter().map(|value| T::from_str(value).map_err(Error::custom)).collect()
}

pub(crate) fn deserialize_string_option<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
    where D: Deserializer<'de>,
          T: FromStr,
          T::Err: Display {
    let value = Option::<String>::deserialize(deserializer)?;
    match value {
        None => Ok(None),
        Some(ref value) => Ok(Some(T::from_str(value).map_err(Error::custom)?)),
    }
}

pub(crate) fn deserialize_tags<'de, D, T>(deserializer: D) -> Result<HashMap<String, T>, D::Error>
    where D: Deserializer<'de>,
          T: FromStr,
          T::Err: Display {
    let str_tags = HashMap::<String, String>::deserialize(deserializer)?;
    let mut tags = HashMap::with_capacity(str_tags.len());
    for (k, v) in str_tags {
        tags.insert(k, T::from_str(&v).map_err(Error::custom)?);
    }
    Ok(tags)
}
