use std::fmt::Display;
use std::str::FromStr;

use serde::{Deserialize, Deserializer};
use serde::de::Error;

use mempool::filter::{MempoolFilter, Rules};
use mempool::MempoolConfig;
use network_primitives::protocol::Protocol;
use primitives::coin::Coin;
use primitives::networks::NetworkId;

use crate::settings as s;
use std::collections::HashMap;

/// Converts protocol from settings into 'normal' protocol
impl From<s::Protocol> for Protocol {
    fn from(protocol: s::Protocol) -> Protocol {
        match protocol {
            s::Protocol::Dumb => Protocol::Dumb,
            s::Protocol::Ws => Protocol::Ws,
            s::Protocol::Wss => Protocol::Wss,
            s::Protocol::Rtc => Protocol::Rtc,
        }
    }
}

/// Converts the network ID from settings into 'normal' network ID
impl From<s::Network> for NetworkId {
    fn from(network: s::Network) -> NetworkId {
        match network {
            s::Network::Main => NetworkId::Main,
            s::Network::Test => NetworkId::Test,
            s::Network::Dev => NetworkId::Dev,
        }
    }
}

/// Convert mempool settings
impl From<s::MempoolSettings> for MempoolConfig {
    fn from(mempool_settings: s::MempoolSettings) -> MempoolConfig {
        let rules = if let Some(f) = mempool_settings.filter {
            Rules {
                tx_fee: f.tx_fee,
                tx_fee_per_byte: f.tx_fee_per_byte,
                tx_value: f.tx_value,
                tx_value_total: f.tx_value_total,
                contract_fee: f.contract_fee,
                contract_fee_per_byte: f.contract_fee_per_byte,
                contract_value: f.contract_value,
                creation_fee: f.creation_fee,
                creation_fee_per_byte: f.creation_fee_per_byte,
                creation_value: f.creation_value,
                sender_balance: f.sender_balance,
                recipient_balance: f.recipient_balance,
            }
        } else { Rules::default() };
        MempoolConfig {
            filter_rules: rules,
            filter_limit: mempool_settings.blacklist_limit.unwrap_or(MempoolFilter::DEFAULT_BLACKLIST_SIZE)
        }
    }
}


pub(crate) fn deserialize_coin<'de, D>(deserializer: D) -> Result<Coin, D::Error> where D: Deserializer<'de> {
    let value = u64::deserialize(deserializer)?;
    Coin::from_u64(value).map_err(Error::custom)
}

pub(crate) fn deserialize_string<'de, D, T>(deserializer: D) -> Result<T, D::Error>
    where D: Deserializer<'de>,
          T: FromStr,
          T::Err: Display {
    let value = String::deserialize(deserializer)?;
    T::from_str(&value).map_err(Error::custom)
}

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
