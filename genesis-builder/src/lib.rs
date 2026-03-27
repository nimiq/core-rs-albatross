#[macro_use]
extern crate log;

use std::{fs, io::Error as IoError, path::Path};

use nimiq_account::{
    Account, Accounts, BasicAccount, HashedTimeLockedContract, StakingContract,
    StakingContractStoreWrite, TransactionLog, VestingContract,
};
use nimiq_block::{Block, MacroBlock, MacroBody, MacroHeader};
use nimiq_bls::PublicKey as BlsPublicKey;
use nimiq_database::{
    mdbx::MdbxDatabase,
    traits::{Database, WriteTransaction},
};
use nimiq_hash::{Blake2bHash, Blake2sHash, Hash};
use nimiq_keys::{Address, Ed25519PublicKey as SchnorrPublicKey};
use nimiq_primitives::{
    account::AccountError,
    coin::Coin,
    key_nibbles::KeyNibbles,
    networks::NetworkId,
    policy::Policy,
    slots_allocation::{Validator, Validators},
    trie::TrieItem,
    TreeProof,
};
use nimiq_serde::{Deserialize, DeserializeError, Serialize};
use nimiq_trie::WriteTransactionProxy;
use nimiq_vrf::VrfSeed;
use thiserror::Error;
use time::OffsetDateTime;
use toml::de::Error as TomlError;

pub mod config;

/// Errors that can be reported building the genesis
#[derive(Debug, Error)]
pub enum GenesisBuilderError {
    /// No VRF seed to generate the genesis block.
    #[error("No VRF seed to generate genesis block")]
    NoVrfSeed,
    /// Serialization failed.
    #[error("Serialization failed: {0}")]
    SerializingError(#[from] DeserializeError),
    /// I/O error
    #[error("I/O error: {0}")]
    IoError(#[from] IoError),
    /// Failure at parsing TOML file
    #[error("Failed to parse TOML file: {0}")]
    TomlError(#[from] TomlError),
    /// Failure at staking
    #[error("Failed to stake: {0}")]
    StakingError(#[from] AccountError),
    /// Data for both thin and full accounts specified
    #[error("Data for both thin and full accounts specified")]
    DataForBothThinAndFullAccounts,
}

/// Output of the Genesis builder that represents the Genesis block and its
/// state.
#[derive(Clone)]
pub struct GenesisInfo {
    /// The genesis block.
    pub block: Block,
    /// The genesis block hash.
    pub hash: Blake2bHash,
    /// The genesis accounts Trie.
    pub accounts: Option<Vec<TrieItem>>,
}

/// Auxiliary struct for generating `GenesisInfo`.
pub struct GenesisBuilder {
    /// The network identification.
    pub network: NetworkId,
    /// The genesis block timestamp.
    pub timestamp: Option<OffsetDateTime>,
    /// The genesis block number.
    pub block_number: u32,
    /// The genesis block VRF seed.
    pub vrf_seed: Option<VrfSeed>,
    /// The parent hash of the genesis block.
    pub parent_hash: Option<Blake2bHash>,
    /// The parent election hash of the genesis block.
    pub parent_election_hash: Option<Blake2bHash>,
    /// Merkle root over all of the transactions previous the genesis block.
    pub history_root: Option<Blake2bHash>,
    pub accounts_data: Option<GenesisBuilderAccounts>,
}

/// Data about accounts at the genesis block.
///
/// There are two ways to specify those: either with all existing accounts,
/// validators and stakers; or by just specifying some metadata.
///
/// You need everything if you want to start a history node for the first time.
///
/// The motivation for using only the accounts metadata is that it's smaller by
/// orders of magnitude, which is helpful if you're not running a history node,
/// e.g. in a web client.
pub enum GenesisBuilderAccounts {
    /// Full accounts data.
    Full(GenesisBuilderFullAccounts),
    /// Only accounts metadata.
    Thin(GenesisBuilderThinAccounts),
}

/// Full genesis accounts data.
///
/// Includes all existing accounts.
#[derive(Default)]
pub struct GenesisBuilderFullAccounts {
    /// The set of validators for the genesis state.
    pub validators: Vec<config::GenesisValidator>,
    /// The set of stakers for the genesis state.
    pub stakers: Vec<config::GenesisStaker>,
    /// The set of basic accounts for the genesis state.
    pub basic_accounts: Vec<config::GenesisAccount>,
    /// The set of vesting accounts for the genesis state.
    pub vesting_accounts: Vec<config::GenesisVestingContract>,
    /// The set of HTLC accounts for the genesis state.
    pub htlc_accounts: Vec<config::GenesisHTLC>,
}

/// Thin genesis accounts data.
///
/// Only contains some metadata and the elected validators for the first epoch.
#[derive(Default)]
pub struct GenesisBuilderThinAccounts {
    /// The total amount of existing coin at the genesis block.
    supply: Coin,
    /// The root of the Merkle tree of the genesis state.
    state_root: Option<Blake2bHash>,
    /// The elected validators for the first epoch after the genesis block.
    slots: Vec<Validator>,
}

trait GenesisBuilderAccountsOption: Sized {
    fn as_accounts_option_mut(&mut self) -> &mut Option<GenesisBuilderAccounts>;

    fn full(&mut self) -> Result<&mut GenesisBuilderFullAccounts, GenesisBuilderError> {
        use GenesisBuilderAccounts::*;
        match self
            .as_accounts_option_mut()
            .get_or_insert_with(|| Full(Default::default()))
        {
            Full(full) => Ok(full),
            Thin(_) => Err(GenesisBuilderError::DataForBothThinAndFullAccounts),
        }
    }
    fn thin(&mut self) -> Result<&mut GenesisBuilderThinAccounts, GenesisBuilderError> {
        use GenesisBuilderAccounts::*;
        match self
            .as_accounts_option_mut()
            .get_or_insert_with(|| Thin(Default::default()))
        {
            Full(_) => Err(GenesisBuilderError::DataForBothThinAndFullAccounts),
            Thin(thin) => Ok(thin),
        }
    }
    #[allow(clippy::ok_expect)]
    fn expect_full(&mut self) -> &mut GenesisBuilderFullAccounts {
        self.as_accounts_option_mut()
            .full()
            .ok()
            .expect("full accounts expected, got thin accounts")
    }
}

impl GenesisBuilderAccountsOption for Option<GenesisBuilderAccounts> {
    fn as_accounts_option_mut(&mut self) -> &mut Option<GenesisBuilderAccounts> {
        self
    }
}

impl<'a> Default for &'a GenesisBuilderAccounts {
    fn default() -> &'a GenesisBuilderAccounts {
        const DEFAULT: &GenesisBuilderAccounts =
            &GenesisBuilderAccounts::Full(GenesisBuilderFullAccounts {
                validators: Vec::new(),
                stakers: Vec::new(),
                basic_accounts: Vec::new(),
                vesting_accounts: Vec::new(),
                htlc_accounts: Vec::new(),
            });
        DEFAULT
    }
}

impl Default for GenesisBuilder {
    fn default() -> Self {
        let mut result = Self::new_without_defaults();
        result.with_defaults();
        result
    }
}

impl GenesisBuilder {
    fn new_without_defaults() -> Self {
        GenesisBuilder {
            network: NetworkId::UnitAlbatross,
            timestamp: None,
            block_number: 0,
            vrf_seed: None,
            parent_election_hash: None,
            parent_hash: None,
            history_root: None,
            accounts_data: None,
        }
    }

    /// Read a genesis config from a TOML config file.
    ///
    /// See `genesis/src/genesis/unit-albatross.toml` for an example.
    pub fn from_config_file<P: AsRef<Path>>(path: P) -> Result<Self, GenesisBuilderError> {
        Self::from_config(toml::from_str(&fs::read_to_string(path)?)?)
    }

    pub fn from_config(config: config::GenesisConfig) -> Result<Self, GenesisBuilderError> {
        let mut result = Self::new_without_defaults();
        result.with_config(config)?;
        Ok(result)
    }

    fn with_defaults(&mut self) -> &mut Self {
        self.vrf_seed = Some(VrfSeed::default());
        self
    }

    /// The network ID for the genesis block.
    ///
    /// Used to distinguish testnet from mainnet and unit tests.
    ///
    /// Sets [`MacroHeader::network`].
    pub fn with_network(&mut self, network: NetworkId) -> &mut Self {
        self.network = network;
        self
    }

    /// The block number for the genesis block.
    ///
    /// Sets [`MacroHeader::block_number`].
    pub fn with_genesis_block_number(&mut self, block_number: u32) -> &mut Self {
        self.block_number = block_number;
        self
    }

    /// The timestamp of the genesis block.
    ///
    /// Sets [`MacroHeader::timestamp`].
    pub fn with_timestamp(&mut self, timestamp: OffsetDateTime) -> &mut Self {
        self.timestamp = Some(timestamp);
        self
    }

    /// The original VRF seed of the genesis block.
    ///
    /// Sets [`MacroHeader::seed`].
    pub fn with_vrf_seed(&mut self, vrf_seed: VrfSeed) -> &mut Self {
        self.vrf_seed = Some(vrf_seed);
        self
    }

    /// The preceding election macro block hash of the genesis block.
    ///
    /// Sets [`MacroHeader::parent_election_hash`].
    pub fn with_parent_election_hash(&mut self, hash: Blake2bHash) -> &mut Self {
        self.parent_election_hash = Some(hash);
        self
    }

    /// The preceding block hash of the genesis block.
    ///
    /// Sets [`MacroHeader::parent_hash`].
    pub fn with_parent_hash(&mut self, hash: Blake2bHash) -> &mut Self {
        self.parent_hash = Some(hash);
        self
    }

    /// The merkle history root of the genesis block.
    ///
    /// Sets [`MacroHeader::history_root`].
    pub fn with_history_root(&mut self, history_root: Blake2bHash) -> &mut Self {
        self.history_root = Some(history_root);
        self
    }

    /// Add a validator to the genesis block.
    pub fn with_genesis_validator(
        &mut self,
        validator_address: Address,
        signing_key: SchnorrPublicKey,
        voting_key: BlsPublicKey,
        reward_address: Address,
        inactive_from: Option<u32>,
        jailed_from: Option<u32>,
        retired: bool,
    ) -> &mut Self {
        self.accounts_data
            .expect_full()
            .validators
            .push(config::GenesisValidator {
                validator_address,
                signing_key,
                voting_key,
                reward_address,
                inactive_from,
                jailed_from,
                retired,
            });
        self
    }

    /// Add a staker to the genesis block.
    pub fn with_genesis_staker(
        &mut self,
        staker_address: Address,
        validator_address: Address,
        balance: Coin,
        inactive_balance: Coin,
        inactive_from: Option<u32>,
    ) -> &mut Self {
        self.accounts_data
            .expect_full()
            .stakers
            .push(config::GenesisStaker {
                staker_address,
                balance,
                delegation: validator_address,
                inactive_balance,
                inactive_from,
            });
        self
    }

    /// Add a basic account with a certain balance to the genesis block.
    pub fn with_basic_account(&mut self, address: Address, balance: Coin) -> &mut Self {
        self.accounts_data
            .expect_full()
            .basic_accounts
            .push(config::GenesisAccount { address, balance });
        self
    }

    fn with_config(
        &mut self,
        config: config::GenesisConfig,
    ) -> Result<&mut Self, GenesisBuilderError> {
        let config::GenesisConfig {
            network,
            timestamp,
            vrf_seed,
            parent_election_hash,
            parent_hash,
            history_root,
            block_number,
            mut validators,
            mut stakers,
            mut basic_accounts,
            mut vesting_accounts,
            mut htlc_accounts,
            supply,
            state_root,
            mut slots,
        } = config;
        self.with_network(network);
        timestamp.map(|t| self.with_timestamp(t));
        self.block_number = block_number;
        vrf_seed.map(|vrf_seed| self.with_vrf_seed(vrf_seed));
        parent_election_hash.map(|hash| self.with_parent_election_hash(hash));
        parent_hash.map(|hash| self.with_parent_hash(hash));
        history_root.map(|history_root| self.with_history_root(history_root));
        if !validators.is_empty() {
            self.accounts_data
                .full()?
                .validators
                .append(&mut validators);
        }
        if !stakers.is_empty() {
            self.accounts_data.full()?.stakers.append(&mut stakers);
        }
        if !basic_accounts.is_empty() {
            self.accounts_data
                .full()?
                .basic_accounts
                .append(&mut basic_accounts);
        }
        if !vesting_accounts.is_empty() {
            self.accounts_data
                .full()?
                .vesting_accounts
                .append(&mut vesting_accounts);
        }
        if !htlc_accounts.is_empty() {
            self.accounts_data
                .full()?
                .htlc_accounts
                .append(&mut htlc_accounts);
        }
        if let Some(supply) = supply {
            self.accounts_data.thin()?.supply = supply;
        }
        if let Some(state_root) = state_root {
            self.accounts_data.thin()?.state_root = Some(state_root);
        }
        if !slots.is_empty() {
            self.accounts_data.thin()?.slots.append(&mut slots);
        }
        Ok(self)
    }

    /// Add a basic account with a certain balance to the genesis block.
    pub fn generate(&self, db: MdbxDatabase) -> Result<GenesisInfo, GenesisBuilderError> {
        // Initialize the environment.
        let timestamp = self.timestamp.unwrap_or_else(OffsetDateTime::now_utc);
        let parent_election_hash = self.parent_election_hash.clone().unwrap_or_default();
        let parent_hash = self.parent_hash.clone().unwrap_or_default();
        let history_root = self.history_root.clone().unwrap_or_default();

        // Generate seeds
        // seed of genesis block = VRF(seed_0)
        let seed = self
            .vrf_seed
            .clone()
            .ok_or(GenesisBuilderError::NoVrfSeed)?;
        debug!(%seed);

        let genesis_accounts;
        let supply;
        let state_root;
        let slots;

        match self.accounts_data.as_ref().unwrap_or_default() {
            GenesisBuilderAccounts::Full(full) => {
                // Initialize the accounts.
                let accounts = Accounts::new(db.clone());

                // Note: This line needs to be AFTER we call Accounts::new().
                let mut raw_txn = db.write_transaction();
                let mut txn = (&mut raw_txn).into();

                debug!("Genesis accounts");
                for genesis_account in &full.basic_accounts {
                    let key = KeyNibbles::from(&genesis_account.address);

                    let account = Account::Basic(BasicAccount {
                        balance: genesis_account.balance,
                    });

                    accounts
                        .tree
                        .put(&mut txn, &key, account)
                        .expect("Failed to store account");
                }

                debug!("Vesting contracts");
                for vesting_contract in &full.vesting_accounts {
                    let key = KeyNibbles::from(&vesting_contract.address);

                    let account = Account::Vesting(VestingContract {
                        balance: vesting_contract.balance,
                        owner: vesting_contract.owner.clone(),
                        start_time: vesting_contract.start_time,
                        step_amount: vesting_contract.step_amount,
                        time_step: vesting_contract.time_step,
                        total_amount: vesting_contract.total_amount,
                    });

                    accounts
                        .tree
                        .put(&mut txn, &key, account)
                        .expect("Failed to store account");
                }

                debug!("HTLC contracts");
                for htlc_contract in &full.htlc_accounts {
                    let key = KeyNibbles::from(&htlc_contract.address);

                    let account = Account::HTLC(HashedTimeLockedContract {
                        balance: htlc_contract.balance,
                        sender: htlc_contract.sender.clone(),
                        recipient: htlc_contract.recipient.clone(),
                        hash_count: htlc_contract.hash_count,
                        hash_root: htlc_contract.hash_root.clone(),
                        timeout: htlc_contract.timeout,
                        total_amount: htlc_contract.total_amount,
                    });

                    accounts
                        .tree
                        .put(&mut txn, &key, account)
                        .expect("Failed to store account");
                }

                debug!("Staking contract");
                // First generate the Staking contract in the Accounts.
                let staking_contract = full.generate_staking_contract(&accounts, &mut txn)?;

                // Update hashes in tree.
                accounts
                    .tree
                    .update_root(&mut txn)
                    .expect("Tree must be complete");

                // Fetch all accounts & contract data items from the tree.
                genesis_accounts = Some(
                    accounts
                        .get_chunk(KeyNibbles::ROOT, usize::MAX - 1, Some(&txn))
                        .items,
                );

                // Generate slot allocation from staking contract.
                let data_store = accounts.data_store(&Policy::STAKING_CONTRACT_ADDRESS);
                slots = staking_contract.select_validators(&data_store.read(&txn), &seed);
                debug!(?slots);

                // State root
                state_root = accounts.get_root_hash_assert(Some(&txn));
                debug!(state_root = %state_root);

                // Supply
                supply = accounts
                    .get_chunk(KeyNibbles::default(), usize::MAX - 1, Some(&txn))
                    .items
                    .into_iter()
                    .filter(|trie| trie.key.to_address().is_some())
                    .map(|trie| Account::deserialize_from_vec(&trie.value).unwrap())
                    .fold(Coin::ZERO, |sum, account| sum + account.balance());
                debug!(initial_supply = %supply);

                raw_txn.abort();
            }
            GenesisBuilderAccounts::Thin(thin) => {
                genesis_accounts = None;
                supply = thin.supply;
                state_root = thin.state_root.clone().unwrap_or_default();
                slots = Validators::new(thin.slots.clone());
            }
        }

        // Body
        let body = MacroBody {
            ..Default::default()
        };

        let body_root = body.hash::<Blake2sHash>();
        debug!(%body_root);

        // The header
        let header = MacroHeader {
            network: self.network,
            version: 1,
            block_number: self.block_number,
            round: 0,
            timestamp: timestamp.unix_timestamp() as u64 * 1000,
            parent_hash,
            parent_election_hash,
            interlink: Some(vec![]),
            seed,
            extra_data: supply.serialize_to_vec(),
            state_root,
            body_root,
            diff_root: TreeProof::empty().root_hash(),
            history_root,
            validators: Some(slots),
            ..Default::default()
        };

        // Genesis hash
        let genesis_hash = header.hash();

        Ok(GenesisInfo {
            block: Block::Macro(MacroBlock {
                header,
                justification: None,
                body: Some(body),
            }),
            hash: genesis_hash,
            accounts: genesis_accounts,
        })
    }
}

impl GenesisBuilderFullAccounts {
    fn generate_staking_contract(
        &self,
        accounts: &Accounts,
        txn: &mut WriteTransactionProxy,
    ) -> Result<StakingContract, GenesisBuilderError> {
        let mut staking_contract = StakingContract::default();

        // Get the deposit value.
        let deposit = Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT);

        let data_store = accounts.data_store(&Policy::STAKING_CONTRACT_ADDRESS);
        let mut data_store_write = data_store.write(txn);
        let mut store = StakingContractStoreWrite::new(&mut data_store_write);

        for validator in &self.validators {
            staking_contract.create_validator(
                &mut store,
                &validator.validator_address,
                validator.signing_key,
                validator.voting_key.compress(),
                validator.reward_address.clone(),
                None,
                deposit,
                validator.inactive_from,
                validator.jailed_from,
                validator.retired,
                &mut TransactionLog::empty(),
            )?;
        }

        for staker in &self.stakers {
            staking_contract.create_staker(
                &mut store,
                &staker.staker_address,
                staker.balance,
                Some(staker.delegation.clone()),
                staker.inactive_balance,
                staker.inactive_from,
                &mut TransactionLog::empty(),
            )?;
        }

        accounts
            .tree
            .put(
                txn,
                &KeyNibbles::from(&Policy::STAKING_CONTRACT_ADDRESS),
                Account::Staking(staking_contract.clone()),
            )
            .expect("Failed to store staking contract");

        Ok(staking_contract)
    }
}

impl GenesisBuilder {
    pub fn write_to_files<P: AsRef<Path>>(
        &self,
        db: MdbxDatabase,
        directory: P,
    ) -> Result<(Blake2bHash, bool), GenesisBuilderError> {
        let directory = directory.as_ref();

        let GenesisInfo {
            block,
            hash,
            accounts,
        } = self.generate(db)?;

        debug!(%hash, "Genesis block");
        debug!(?block);
        debug!("Accounts:");
        debug!(?accounts);

        let block_path = directory.join("block.dat");
        info!(path = %block_path.display(), "Writing block to");
        fs::write(block_path, block.serialize_to_vec())?;

        {
            let decompressed_path = directory.join("decompressed_keys.dat");
            let mut decompressed = Vec::new();
            for key in block
                .validators()
                .expect("must be election")
                .voting_keys()
                .expect("genesis validators must have valid keys")
            {
                decompressed.extend_from_slice(&key.trusted_serialize());
            }
            fs::write(decompressed_path, decompressed)?;
        }

        let have_accounts = accounts.is_some();
        if let Some(accounts) = accounts {
            let accounts_path = directory.join("accounts.dat");
            info!(path = %accounts_path.display(), "Writing accounts to");
            fs::write(accounts_path, accounts.serialize_to_vec())?;
        }

        Ok((hash, have_accounts))
    }
}
