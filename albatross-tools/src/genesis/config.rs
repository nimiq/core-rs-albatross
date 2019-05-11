use chrono::{DateTime, Utc};
use keys::Address;
use serde::{Deserialize, Deserializer};
use serde::de::Error;
use primitives::coin::Coin;
use bls::bls12_381::{
    PublicKey as BlsPublicKey,
    SecretKey as BlsSecretKey,
};
use bls::Encoding;
use std::convert::TryFrom;
use std::fs::read_to_string;
use std::path::Path;


#[derive(Clone, Debug, Deserialize)]
pub struct GenesisConfig {
    pub seed_message: String,
    pub timestamp: Option<DateTime<Utc>>,
    #[serde(default)]
    pub stakes: Vec<GenesisStake>,
    #[serde(default)]
    pub accounts: Vec<GenesisAccount>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct GenesisStake {
    #[serde(deserialize_with = "deserialize_nimiq_address")]
    pub staker_address: Address,
    #[serde(deserialize_with = "deserialize_nimiq_address")]
    pub reward_address: Address,
    //#[serde(deserialize_with = "deserialize_coin")]
    pub balance: u64,
    #[serde(deserialize_with = "deserialize_bls_public_key")]
    pub validator_key: BlsPublicKey
}

#[derive(Clone, Debug, Deserialize)]
pub struct GenesisAccount {
    #[serde(deserialize_with = "deserialize_nimiq_address")]
    pub address: Address,
    //#[serde(deserialize_with = "deserialize_coin")]
    pub balance: u64,
}

impl GenesisConfig {
    pub fn from_file<P: AsRef<Path>>(path: P) -> GenesisConfig {
        toml::from_str(&read_to_string(path).expect("Invalid file path"))
            .expect("Failed to load genesis configuration")
    }
}


pub fn deserialize_nimiq_address<'de, D>(deserializer: D) -> Result<Address, D::Error> where D: Deserializer<'de> {
    let s = String::deserialize(deserializer)?;
    Address::from_user_friendly_address(&s).map_err(|e| Error::custom(format!("{:?}", e)))
}

pub(crate) fn deserialize_coin<'de, D>(deserializer: D) -> Result<Coin, D::Error> where D: Deserializer<'de> {
    let value = u64::deserialize(deserializer)?;
    Coin::try_from(value).map_err(Error::custom)
}

pub(crate) fn deserialize_bls_public_key<'de, D>(deserializer: D) -> Result<BlsPublicKey, D::Error> where D: Deserializer<'de> {
    let pkey_hex = String::deserialize(deserializer)?;
    let pkey_raw = hex::decode(pkey_hex).map_err(Error::custom)?;
    BlsPublicKey::from_slice(&pkey_raw).map_err(Error::custom)
}

pub(crate) fn deserialize_bls_secret_key<'de, D>(deserializer: D) -> Result<BlsSecretKey, D::Error> where D: Deserializer<'de> {
    let skey_hex = String::deserialize(deserializer)?;
    let skey_raw = hex::decode(skey_hex).map_err(Error::custom)?;
    BlsSecretKey::from_slice(&skey_raw).map_err(Error::custom)
}
