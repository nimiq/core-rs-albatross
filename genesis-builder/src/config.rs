use std::convert::TryFrom;

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
    pub validator_address: Address,

    pub signing_key: SchnorrPublicKey,

    pub voting_key: BlsPublicKey,

    pub reward_address: Address,
}

#[derive(Clone, Debug, Deserialize)]
pub struct GenesisStaker {
    pub staker_address: Address,

    #[serde(deserialize_with = "deserialize_coin")]
    pub balance: Coin,

    pub delegation: Address,
}

#[derive(Clone, Debug, Deserialize)]
pub struct GenesisAccount {
    pub address: Address,

    #[serde(deserialize_with = "deserialize_coin")]
    pub balance: Coin,
}

/// Struct that represents a vesting contract in the toml file that is used to generate the genesis
#[derive(Clone, Debug, Deserialize)]
pub struct GenesisVestingContract {
    /// Vesting contract account address
    pub address: Address,
    /// The one who owns the vesting contract
    pub owner: Address,
    /// Vesting contract balance
    #[serde(deserialize_with = "deserialize_coin")]
    pub balance: Coin,
    /// Vesting contract start time
    pub start_time: u64,
    /// Vesting contract time step
    pub time_step: u64,
    #[serde(deserialize_with = "deserialize_coin")]
    /// Vesting contract step amount
    pub step_amount: Coin,
    /// Vesting contract total amount
    #[serde(deserialize_with = "deserialize_coin")]
    pub total_amount: Coin,
}

/// Struct that represents an HTLC in the toml file that is used to generate the genesis
#[derive(Clone, Debug, Deserialize)]
pub struct GenesisHTLC {
    /// HTLC account address
    pub address: Address,
    /// The one who sent the HTLC
    pub sender: Address,
    /// The recipient of the HTLC
    pub recipient: Address,
    /// HTLC coin balance
    #[serde(deserialize_with = "deserialize_coin")]
    pub balance: Coin,
    /// HTLC hashing algorithm
    pub hash_algorithm: HashAlgorithm,
    /// HTLC hash root
    pub hash_root: AnyHash,
    /// HTLC hash count
    pub hash_count: u8,
    /// HTLC timeout
    pub timeout: u64,
    /// HTLC total amount
    pub total_amount: Coin,
}

pub(crate) fn deserialize_coin<'de, D>(deserializer: D) -> Result<Coin, D::Error>
where
    D: Deserializer<'de>,
{
    let value: u64 = Deserialize::deserialize(deserializer)?;
    Coin::try_from(value).map_err(Error::custom)
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
