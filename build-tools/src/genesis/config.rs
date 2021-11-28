use std::convert::TryFrom;

use serde::de::Error;
use serde::{Deserialize, Deserializer};
use time::OffsetDateTime;

use beserial::Deserialize as BDeserialize;
use bls::PublicKey as BlsPublicKey;
use keys::{Address, PrivateKey as SchnorrPrivateKey, PublicKey as SchnorrPublicKey};
use primitives::coin::Coin;

#[derive(Clone, Debug, Deserialize)]
pub struct GenesisConfig {
    #[serde(default)]
    #[serde(deserialize_with = "deserialize_schnorr_secret_key_opt")]
    pub signing_key: Option<SchnorrPrivateKey>,

    pub seed_message: Option<String>,

    #[serde(deserialize_with = "deserialize_timestamp")]
    pub timestamp: Option<OffsetDateTime>,

    #[serde(default)]
    pub validators: Vec<GenesisValidator>,

    #[serde(default)]
    pub stakers: Vec<GenesisStaker>,

    #[serde(default)]
    pub accounts: Vec<GenesisAccount>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct GenesisValidator {
    #[serde(deserialize_with = "deserialize_nimiq_address")]
    pub validator_address: Address,

    #[serde(deserialize_with = "deserialize_schnorr_public_key")]
    pub signing_key: SchnorrPublicKey,

    #[serde(deserialize_with = "deserialize_bls_public_key")]
    pub voting_key: BlsPublicKey,

    #[serde(deserialize_with = "deserialize_nimiq_address")]
    pub reward_address: Address,
}

#[derive(Clone, Debug, Deserialize)]
pub struct GenesisStaker {
    #[serde(deserialize_with = "deserialize_nimiq_address")]
    pub staker_address: Address,

    #[serde(deserialize_with = "deserialize_coin")]
    pub balance: Coin,

    #[serde(deserialize_with = "deserialize_nimiq_address")]
    pub delegation: Address,
}

#[derive(Clone, Debug, Deserialize)]
pub struct GenesisAccount {
    #[serde(deserialize_with = "deserialize_nimiq_address")]
    pub address: Address,

    #[serde(deserialize_with = "deserialize_coin")]
    pub balance: Coin,
}

pub fn deserialize_nimiq_address<'de, D>(deserializer: D) -> Result<Address, D::Error>
where
    D: Deserializer<'de>,
{
    let s: String = Deserialize::deserialize(deserializer)?;
    Address::from_user_friendly_address(&s).map_err(|e| Error::custom(format!("{:?}", e)))
}

#[allow(dead_code)]
pub fn deserialize_nimiq_address_opt<'de, D>(deserializer: D) -> Result<Option<Address>, D::Error>
where
    D: Deserializer<'de>,
{
    let opt: Option<String> = Deserialize::deserialize(deserializer)?;
    if let Some(s) = opt {
        Ok(Some(
            Address::from_user_friendly_address(&s)
                .map_err(|e| Error::custom(format!("{:?}", e)))?,
        ))
    } else {
        Ok(None)
    }
}

pub(crate) fn deserialize_coin<'de, D>(deserializer: D) -> Result<Coin, D::Error>
where
    D: Deserializer<'de>,
{
    let value: u64 = Deserialize::deserialize(deserializer)?;
    Coin::try_from(value).map_err(Error::custom)
}

pub(crate) fn deserialize_bls_public_key<'de, D>(deserializer: D) -> Result<BlsPublicKey, D::Error>
where
    D: Deserializer<'de>,
{
    let pkey_hex: String = Deserialize::deserialize(deserializer)?;
    let pkey_raw = hex::decode(pkey_hex).map_err(Error::custom)?;
    BlsPublicKey::deserialize_from_vec(&pkey_raw).map_err(Error::custom)
}

pub(crate) fn deserialize_schnorr_secret_key_opt<'de, D>(
    deserializer: D,
) -> Result<Option<SchnorrPrivateKey>, D::Error>
where
    D: Deserializer<'de>,
{
    let opt: Option<String> = Deserialize::deserialize(deserializer)?;
    if let Some(skey_hex) = opt {
        let skey_raw = hex::decode(skey_hex).map_err(Error::custom)?;
        Ok(Some(
            SchnorrPrivateKey::deserialize_from_vec(&skey_raw).map_err(Error::custom)?,
        ))
    } else {
        Ok(None)
    }
}

pub(crate) fn deserialize_schnorr_public_key<'de, D>(
    deserializer: D,
) -> Result<SchnorrPublicKey, D::Error>
where
    D: Deserializer<'de>,
{
    let pkey_hex: String = Deserialize::deserialize(deserializer)?;
    let pkey_raw = hex::decode(pkey_hex).map_err(Error::custom)?;
    SchnorrPublicKey::deserialize_from_vec(&pkey_raw).map_err(Error::custom)
}

pub fn deserialize_timestamp<'de, D>(deserializer: D) -> Result<Option<OffsetDateTime>, D::Error>
where
    D: Deserializer<'de>,
{
    let opt: Option<String> = Deserialize::deserialize(deserializer)?;
    if let Some(s) = opt {
        Ok(Some(
            OffsetDateTime::parse(&s, &time::format_description::well_known::Rfc3339)
                .map_err(|e| Error::custom(format!("{:?}", e)))?,
        ))
    } else {
        Ok(None)
    }
}
