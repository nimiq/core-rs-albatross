use nimiq_bls::PublicKey as BlsPublicKey;
use nimiq_hash::Blake2bHash;
use nimiq_keys::{Address, Ed25519PublicKey as SchnorrPublicKey};
use nimiq_primitives::{coin::Coin, networks::NetworkId};
use nimiq_serde::{Deserialize, Serialize};
use nimiq_transaction::account::htlc_contract::AnyHash;
use nimiq_vrf::VrfSeed;
use time::OffsetDateTime;

/// Struct that defines the genesis configuration that is going to be parsed
/// from the genesis TOML files.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GenesisConfig {
    /// Network ID used in blocks, transactions, etc.
    pub network: NetworkId,

    /// Timestamp for the genesis block.
    #[serde(with = "time::serde::rfc3339::option")]
    pub timestamp: Option<OffsetDateTime>,

    /// VRF seed for the genesis block.
    pub vrf_seed: Option<VrfSeed>,

    /// Hash of the parent election block for the genesis block.
    pub parent_election_hash: Option<Blake2bHash>,

    /// Hash of the parent block for the genesis block.
    pub parent_hash: Option<Blake2bHash>,

    /// Merkle root over all of the transactions previous the genesis block.
    pub history_root: Option<Blake2bHash>,

    /// The genesis block number.
    #[serde(default)]
    pub block_number: u32,

    /// The set of validators for the genesis state.
    #[serde(default)]
    pub validators: Vec<GenesisValidator>,

    /// The set of stakers for the genesis state.
    #[serde(default)]
    pub stakers: Vec<GenesisStaker>,

    /// Set of basic accounts for the genesis state.
    #[serde(default)]
    pub basic_accounts: Vec<GenesisAccount>,

    /// Set of vesting accounts for the genesis state.
    #[serde(default)]
    pub vesting_accounts: Vec<GenesisVestingContract>,

    /// Set of HTLC accounts for the genesis state.
    #[serde(default)]
    pub htlc_accounts: Vec<GenesisHTLC>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GenesisValidator {
    pub validator_address: Address,
    pub signing_key: SchnorrPublicKey,
    pub voting_key: BlsPublicKey,
    pub reward_address: Address,
    pub inactive_from: Option<u32>,
    pub jailed_from: Option<u32>,
    #[serde(default)]
    pub retired: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GenesisStaker {
    pub staker_address: Address,
    pub balance: Coin,
    pub delegation: Address,
    #[serde(default)]
    pub inactive_balance: Coin,
    pub inactive_from: Option<u32>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GenesisAccount {
    pub address: Address,
    pub balance: Coin,
}

/// Struct that represents a vesting contract in the toml file that is used to generate the genesis
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GenesisVestingContract {
    /// Vesting contract account address
    pub address: Address,
    /// The one who owns the vesting contract
    pub owner: Address,
    /// Vesting contract balance
    pub balance: Coin,
    /// Vesting contract start time
    pub start_time: u64,
    /// Vesting contract time step
    pub time_step: u64,
    /// Vesting contract step amount
    pub step_amount: Coin,
    /// Vesting contract total amount
    pub total_amount: Coin,
}

/// Struct that represents an HTLC in the toml file that is used to generate the genesis
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GenesisHTLC {
    /// HTLC account address
    pub address: Address,
    /// The one who sent the HTLC
    pub sender: Address,
    /// The recipient of the HTLC
    pub recipient: Address,
    /// HTLC coin balance
    pub balance: Coin,
    /// HTLC hash root
    pub hash_root: AnyHash,
    /// HTLC hash count
    pub hash_count: u8,
    /// HTLC timeout
    pub timeout: u64,
    /// HTLC total amount
    pub total_amount: Coin,
}
