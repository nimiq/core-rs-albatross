use std::convert::TryFrom;

use beserial::Deserialize as BDeserialize;
use nimiq_bls::PublicKey as BlsPublicKey;
use nimiq_keys::{Address, PublicKey as SchnorrPublicKey};
use nimiq_primitives::coin::Coin;
use nimiq_transaction::account::htlc_contract::{AnyHash, HashAlgorithm};
use nimiq_vrf::VrfSeed;
use serde::{de::Error, Deserialize, Deserializer};
use time::OffsetDateTime;

#[derive(Clone, Debug, Deserialize)]
pub struct GenesisConfig {
    #[serde(default)]
    pub seed_message: Option<String>,

    #[serde(deserialize_with = "deserialize_vrf_seed_opt")]
    pub vrf_seed: Option<VrfSeed>,

    #[serde(deserialize_with = "deserialize_timestamp")]
    pub timestamp: Option<OffsetDateTime>,

    #[serde(default)]
    pub validators: Vec<GenesisValidator>,

    #[serde(default)]
    pub stakers: Vec<GenesisStaker>,

    #[serde(default)]
    pub basic_accounts: Vec<GenesisAccount>,

    #[serde(default)]
    pub vesting_accounts: Vec<GenesisVestingContract>,

    #[serde(default)]
    pub htlc_accounts: Vec<GenesisHTLC>,
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

#[derive(Clone, Debug, Deserialize)]
pub struct GenesisVestingContract {
    #[serde(deserialize_with = "deserialize_nimiq_address")]
    pub address: Address,
    #[serde(deserialize_with = "deserialize_nimiq_address")]
    pub owner: Address,
    #[serde(deserialize_with = "deserialize_coin")]
    pub balance: Coin,
    pub start_time: u64,
    pub time_step: u64,
    #[serde(deserialize_with = "deserialize_coin")]
    pub step_amount: Coin,
    #[serde(deserialize_with = "deserialize_coin")]
    pub total_amount: Coin,
}

#[derive(Clone, Debug, Deserialize)]
pub struct GenesisHTLC {
    #[serde(deserialize_with = "deserialize_nimiq_address")]
    pub address: Address,
    #[serde(deserialize_with = "deserialize_nimiq_address")]
    pub sender: Address,
    #[serde(deserialize_with = "deserialize_nimiq_address")]
    pub recipient: Address,
    #[serde(deserialize_with = "deserialize_coin")]
    pub balance: Coin,

    pub hash_algorithm: HashAlgorithm,
    pub hash_root: AnyHash,
    pub hash_count: u8,
    pub timeout: u64,
    pub total_amount: Coin,
}

pub fn deserialize_nimiq_address<'de, D>(deserializer: D) -> Result<Address, D::Error>
where
    D: Deserializer<'de>,
{
    let s: String = Deserialize::deserialize(deserializer)?;
    Address::from_user_friendly_address(&s).map_err(|e| Error::custom(format!("{e:?}")))
}

#[allow(dead_code)]
pub fn deserialize_nimiq_address_opt<'de, D>(deserializer: D) -> Result<Option<Address>, D::Error>
where
    D: Deserializer<'de>,
{
    let opt: Option<String> = Deserialize::deserialize(deserializer)?;
    if let Some(s) = opt {
        Ok(Some(
            Address::from_user_friendly_address(&s).map_err(|e| Error::custom(format!("{e:?}")))?,
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
                .map_err(|e| Error::custom(format!("{e:?}")))?,
        ))
    } else {
        Ok(None)
    }
}

pub(crate) fn deserialize_vrf_seed_opt<'de, D>(deserializer: D) -> Result<Option<VrfSeed>, D::Error>
where
    D: Deserializer<'de>,
{
    let vrf_seed_hex: Option<String> = Deserialize::deserialize(deserializer)?;
    if let Some(vrf_seed_hex) = vrf_seed_hex {
        let vrf_seed_raw = hex::decode(vrf_seed_hex).map_err(Error::custom)?;
        Ok(Some(
            VrfSeed::deserialize_from_vec(&vrf_seed_raw).map_err(Error::custom)?,
        ))
    } else {
        Ok(None)
    }
}
