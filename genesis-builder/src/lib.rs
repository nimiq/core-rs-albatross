#[macro_use]
extern crate log;

use std::convert::TryFrom;
use std::fs::{read_to_string, OpenOptions};
use std::io::Error as IoError;
use std::path::Path;

use thiserror::Error;
use time::OffsetDateTime;
use toml::de::Error as TomlError;

use beserial::{Serialize, SerializeWithLength, SerializingError};
use nimiq_account::{
    Account, Accounts, BasicAccount, StakingContract, StakingContractStoreWrite, TransactionLog,
};
use nimiq_block::{Block, MacroBlock, MacroBody, MacroHeader};
use nimiq_bls::PublicKey as BlsPublicKey;
use nimiq_database::{
    traits::{Database, WriteTransaction},
    DatabaseProxy, WriteTransactionProxy,
};
use nimiq_hash::{Blake2bHash, Hash};
use nimiq_keys::{Address, PublicKey as SchnorrPublicKey};
use nimiq_primitives::{
    account::AccountError, coin::Coin, key_nibbles::KeyNibbles, policy::Policy, trie::TrieItem,
};
use nimiq_vrf::VrfSeed;

mod config;

#[derive(Debug, Error)]
pub enum GenesisBuilderError {
    #[error("No VRF seed to generate genesis block")]
    NoVrfSeed,
    #[error("Invalid timestamp: {0}")]
    InvalidTimestamp(OffsetDateTime),
    #[error("Serialization failed")]
    SerializingError(#[from] SerializingError),
    #[error("I/O error")]
    IoError(#[from] IoError),
    #[error("Failed to parse TOML file")]
    TomlError(#[from] TomlError),
    #[error("Failed to stake")]
    StakingError(#[from] AccountError),
}

#[derive(Clone)]
pub struct GenesisInfo {
    pub block: Block,
    pub hash: Blake2bHash,
    pub accounts: Vec<TrieItem>,
}

pub struct GenesisBuilder {
    pub seed_message: Option<String>,
    pub timestamp: Option<OffsetDateTime>,
    pub vrf_seed: Option<VrfSeed>,
    pub validators: Vec<config::GenesisValidator>,
    pub stakers: Vec<config::GenesisStaker>,
    pub accounts: Vec<config::GenesisAccount>,
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
            validators: vec![],
            stakers: vec![],
            accounts: vec![],
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
        self.accounts
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
            mut validators,
            mut stakers,
            mut accounts,
        } = toml::from_str(&read_to_string(path)?)?;
        vrf_seed.map(|vrf_seed| self.with_vrf_seed(vrf_seed));
        seed_message.map(|msg| self.with_seed_message(msg));
        timestamp.map(|t| self.with_timestamp(t));
        self.validators.append(&mut validators);
        self.stakers.append(&mut stakers);
        self.accounts.append(&mut accounts);

        Ok(self)
    }

    pub fn generate(&self, env: DatabaseProxy) -> Result<GenesisInfo, GenesisBuilderError> {
        // Initialize the environment.
        let timestamp = self.timestamp.unwrap_or_else(OffsetDateTime::now_utc);

        // Initialize the accounts.
        let accounts = Accounts::new(env.clone());

        // Note: This line needs to be AFTER we call Accounts::new().
        let mut txn = env.write_transaction();

        debug!("Genesis accounts");
        for genesis_account in &self.accounts {
            let key = KeyNibbles::from(&genesis_account.address);

            let account = Account::Basic(BasicAccount {
                balance: genesis_account.balance,
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
        let pk_tree_root = MacroBlock::calc_pk_tree_root(&slots).unwrap();
        debug!("Slots: {:#?}", slots);

        // Body
        let body = MacroBody {
            validators: Some(slots),
            pk_tree_root: Some(pk_tree_root),
            ..Default::default()
        };

        let body_root = body.hash::<Blake2bHash>();
        debug!("Body root: {}", &body_root);

        // State root
        let state_root = accounts.get_root_hash_assert(Some(&txn));
        debug!("State root: {}", &state_root);
        txn.abort();

        // The header
        let header = MacroHeader {
            version: 1,
            block_number: 0,
            round: 0,
            timestamp: u64::try_from(timestamp.unix_timestamp())
                .map_err(|_| GenesisBuilderError::InvalidTimestamp(timestamp))?,
            parent_hash: [0u8; 32].into(),
            parent_election_hash: [0u8; 32].into(),
            interlink: Some(vec![]),
            seed,
            extra_data: vec![],
            state_root,
            body_root,
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
        block.serialize(&mut file)?;

        let accounts_path = directory.as_ref().join("accounts.dat");
        info!("Writing accounts to {}", accounts_path.display());
        let mut file = OpenOptions::new()
            .create(true)
            .write(true)
            .open(&accounts_path)?;
        accounts.serialize::<u32, _>(&mut file)?;

        Ok(hash)
    }
}
