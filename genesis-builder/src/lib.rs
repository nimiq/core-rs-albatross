#[macro_use]
extern crate log;

use std::{
    fs::{read_to_string, OpenOptions},
    io::Error as IoError,
    path::Path,
};

use nimiq_account::{
    Account, Accounts, BasicAccount, HashedTimeLockedContract, StakingContract,
    StakingContractStoreWrite, TransactionLog, VestingContract,
};
use nimiq_block::{Block, MacroBlock, MacroBody, MacroHeader};
use nimiq_bls::PublicKey as BlsPublicKey;
use nimiq_database::{
    traits::{Database, WriteTransaction},
    DatabaseProxy,
};
use nimiq_hash::{Blake2bHash, Blake2sHash, Hash};
use nimiq_keys::{Address, PublicKey as SchnorrPublicKey};
use nimiq_primitives::{
    account::AccountError, coin::Coin, key_nibbles::KeyNibbles, policy::Policy, trie::TrieItem,
    TreeProof,
};
use nimiq_serde::{DeserializeError, Serialize};
use nimiq_trie::WriteTransactionProxy;
use nimiq_vrf::VrfSeed;
use thiserror::Error;
use time::OffsetDateTime;
use toml::de::Error as TomlError;

mod config;

/// Errors that can be reported building the genesis
#[derive(Debug, Error)]
pub enum GenesisBuilderError {
    /// No VRF seed to generate the genesis block.
    #[error("No VRF seed to generate genesis block")]
    NoVrfSeed,
    /// Serialization failed.
    #[error("Serialization failed")]
    SerializingError(#[from] DeserializeError),
    /// I/O error
    #[error("I/O error")]
    IoError(#[from] IoError),
    /// Failure at parsing TOML file
    #[error("Failed to parse TOML file")]
    TomlError(#[from] TomlError),
    /// Failure at staking
    #[error("Failed to stake")]
    StakingError(#[from] AccountError),
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
    pub accounts: Vec<TrieItem>,
}

/// Auxiliary struct for generating `GenesisInfo`.
pub struct GenesisBuilder {
    /// The genesis seed message.
    pub seed_message: Option<String>,
    /// The genesis block timestamp.
    pub timestamp: Option<OffsetDateTime>,
    /// The genesis block VRF seed.
    pub vrf_seed: Option<VrfSeed>,
    /// The parent hash of the genesis block.
    pub parent_hash: Option<Blake2bHash>,
    /// The parent election hash of the genesis block.
    pub parent_election_hash: Option<Blake2bHash>,
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
            seed_message: None,
            timestamp: None,
            vrf_seed: None,
            parent_election_hash: None,
            parent_hash: None,
            validators: vec![],
            stakers: vec![],
            basic_accounts: vec![],
            vesting_accounts: vec![],
            htlc_accounts: vec![],
        }
    }

    pub fn from_config_file<P: AsRef<Path>>(path: P) -> Result<Self, GenesisBuilderError> {
        let mut result = Self::new_without_defaults();
        result.with_config_file(path)?;
        Ok(result)
    }

    fn with_defaults(&mut self) -> &mut Self {
        self.vrf_seed = Some(VrfSeed::default());
        self
    }

    pub fn with_seed_message<S: AsRef<str>>(&mut self, seed_message: S) -> &mut Self {
        self.seed_message = Some(seed_message.as_ref().to_string());
        self
    }

    pub fn with_timestamp(&mut self, timestamp: OffsetDateTime) -> &mut Self {
        self.timestamp = Some(timestamp);
        self
    }

    pub fn with_vrf_seed(&mut self, vrf_seed: VrfSeed) -> &mut Self {
        self.vrf_seed = Some(vrf_seed);
        self
    }

    pub fn with_parent_election_hash(&mut self, hash: Blake2bHash) -> &mut Self {
        self.parent_election_hash = Some(hash);
        self
    }

    pub fn with_parent_hash(&mut self, hash: Blake2bHash) -> &mut Self {
        self.parent_hash = Some(hash);
        self
    }

    pub fn with_genesis_validator(
        &mut self,
        validator_address: Address,
        signing_key: SchnorrPublicKey,
        voting_key: BlsPublicKey,
        reward_address: Address,
    ) -> &mut Self {
        self.validators.push(config::GenesisValidator {
            validator_address,
            signing_key,
            voting_key,
            reward_address,
        });
        self
    }

    pub fn with_genesis_staker(
        &mut self,
        staker_address: Address,
        validator_address: Address,
        balance: Coin,
    ) -> &mut Self {
        self.stakers.push(config::GenesisStaker {
            staker_address,
            balance,
            delegation: validator_address,
        });
        self
    }

    pub fn with_basic_account(&mut self, address: Address, balance: Coin) -> &mut Self {
        self.basic_accounts
            .push(config::GenesisAccount { address, balance });
        self
    }

    fn with_config_file<P: AsRef<Path>>(
        &mut self,
        path: P,
    ) -> Result<&mut Self, GenesisBuilderError> {
        let config::GenesisConfig {
            seed_message,
            timestamp,
            vrf_seed,
            parent_election_hash,
            parent_hash,
            mut validators,
            mut stakers,
            mut basic_accounts,
            mut vesting_accounts,
            mut htlc_accounts,
        } = toml::from_str(&read_to_string(path)?)?;
        vrf_seed.map(|vrf_seed| self.with_vrf_seed(vrf_seed));
        seed_message.map(|msg| self.with_seed_message(msg));
        timestamp.map(|t| self.with_timestamp(t));
        parent_election_hash.map(|hash| self.with_parent_election_hash(hash));
        parent_hash.map(|hash| self.with_parent_hash(hash));
        self.validators.append(&mut validators);
        self.stakers.append(&mut stakers);
        self.basic_accounts.append(&mut basic_accounts);
        self.vesting_accounts.append(&mut vesting_accounts);
        self.htlc_accounts.append(&mut htlc_accounts);

        Ok(self)
    }

    pub fn generate(&self, env: DatabaseProxy) -> Result<GenesisInfo, GenesisBuilderError> {
        // Initialize the environment.
        let timestamp = self.timestamp.unwrap_or_else(OffsetDateTime::now_utc);
        let parent_election_hash = self.parent_election_hash.clone().unwrap_or_default();
        let parent_hash = self.parent_hash.clone().unwrap_or_default();

        // Initialize the accounts.
        let accounts = Accounts::new(env.clone());

        // Note: This line needs to be AFTER we call Accounts::new().
        let mut raw_txn = env.write_transaction();
        let mut txn = (&mut raw_txn).into();

        debug!("Genesis accounts");
        for genesis_account in &self.basic_accounts {
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
        for vesting_contract in &self.vesting_accounts {
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
        for htlc_contract in &self.htlc_accounts {
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
        let staking_contract = self.generate_staking_contract(&accounts, &mut txn)?;

        // Update hashes in tree.
        accounts
            .tree
            .update_root(&mut txn)
            .expect("Tree must be complete");

        // Fetch all accounts & contract data items from the tree.
        let genesis_accounts = accounts
            .get_chunk(KeyNibbles::ROOT, usize::MAX - 1, Some(&txn))
            .items;

        // Generate seeds
        // seed of genesis block = VRF(seed_0)
        let seed = self
            .vrf_seed
            .clone()
            .ok_or(GenesisBuilderError::NoVrfSeed)?;
        debug!("Genesis seed: {}", seed);

        // Generate slot allocation from staking contract.
        let data_store = accounts.data_store(&Policy::STAKING_CONTRACT_ADDRESS);
        let slots = staking_contract.select_validators(&data_store.read(&txn), &seed);
        debug!("Slots: {:#?}", slots);

        // Body
        let body = MacroBody {
            validators: Some(slots),
            ..Default::default()
        };

        let body_root = body.hash::<Blake2sHash>();
        debug!("Body root: {}", &body_root);

        // State root
        let state_root = accounts.get_root_hash_assert(Some(&txn));
        debug!("State root: {}", &state_root);

        // Supply
        let supply = accounts
            .tree
            .iter_nodes::<Account>(
                &txn,
                &KeyNibbles::from(&Address::START_ADDRESS),
                &KeyNibbles::from(&Address::END_ADDRESS),
            )
            .fold(Coin::ZERO, |sum, account| sum + account.balance());

        raw_txn.abort();

        // The header
        let header = MacroHeader {
            version: 1,
            block_number: 0,
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
            history_root: Blake2bHash::default(),
        };

        // Genesis hash
        let genesis_hash = header.hash::<Blake2bHash>();

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
                &mut TransactionLog::empty(),
            )?;
        }

        for staker in &self.stakers {
            staking_contract.create_staker(
                &mut store,
                &staker.staker_address,
                staker.balance,
                Some(staker.delegation.clone()),
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

    pub fn write_to_files<P: AsRef<Path>>(
        &self,
        env: DatabaseProxy,
        directory: P,
    ) -> Result<Blake2bHash, GenesisBuilderError> {
        let GenesisInfo {
            block,
            hash,
            accounts,
        } = self.generate(env)?;

        debug!("Genesis block: {}", &hash);
        debug!("{:#?}", &block);
        debug!("Accounts:");
        debug!("{:#?}", &accounts);

        let block_path = directory.as_ref().join("block.dat");
        info!("Writing block to {}", block_path.display());
        let mut file = OpenOptions::new()
            .create(true)
            .write(true)
            .open(&block_path)?;
        block.serialize_to_writer(&mut file)?;

        let accounts_path = directory.as_ref().join("accounts.dat");
        info!("Writing accounts to {}", accounts_path.display());
        let mut file = OpenOptions::new()
            .create(true)
            .write(true)
            .open(&accounts_path)?;
        accounts.serialize_to_writer(&mut file)?;

        Ok(hash)
    }
}
