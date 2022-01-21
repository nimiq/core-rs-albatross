use std::convert::TryFrom;
use std::fs::{read_to_string, OpenOptions};
use std::io::Error as IoError;
use std::path::Path;

use thiserror::Error;
use time::OffsetDateTime;
use toml::de::Error as TomlError;

use account::{Account, AccountError, Accounts, AccountsList, BasicAccount, StakingContract};
use beserial::{Serialize, SerializingError};
use block::{Block, MacroBlock, MacroBody, MacroHeader};
use bls::PublicKey as BlsPublicKey;
use database::Environment;
use database::WriteTransaction;
use hash::{Blake2bHash, Hash};
use keys::{Address, PublicKey as SchnorrPublicKey};
use nimiq_trie::key_nibbles::KeyNibbles;
use primitives::coin::Coin;
use vrf::VrfSeed;

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
    pub accounts: Vec<(KeyNibbles, Account)>,
}

pub struct GenesisBuilder {
    pub seed_message: Option<String>,
    pub timestamp: Option<OffsetDateTime>,
    pub vrf_seed: Option<VrfSeed>,
    pub validators: Vec<config::GenesisValidator>,
    pub stakers: Vec<config::GenesisStaker>,
    pub accounts: Vec<config::GenesisAccount>,
}

impl GenesisBuilder {
    pub fn new() -> Self {
        GenesisBuilder {
            seed_message: None,
            timestamp: None,
            vrf_seed: None,
            validators: vec![],
            stakers: vec![],
            accounts: vec![],
        }
    }

    pub fn default() -> Self {
        let mut builder = Self::new();
        builder.with_defaults();
        builder
    }

    pub fn with_defaults(&mut self) -> &mut Self {
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

    pub fn with_config_file<P: AsRef<Path>>(
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

    pub fn generate(&self, env: Environment) -> Result<GenesisInfo, GenesisBuilderError> {
        // Initialize the environment.
        let timestamp = self.timestamp.unwrap_or_else(OffsetDateTime::now_utc);

        // Initialize the accounts.
        let accounts = Accounts::new(env.clone());
        let mut genesis_accounts: Vec<(KeyNibbles, Account)> = Vec::new();

        // Note: This line needs to be AFTER we call Accounts::new().
        let mut txn = WriteTransaction::new(&env);

        debug!("Genesis accounts");
        for genesis_account in &self.accounts {
            let key = KeyNibbles::from(&genesis_account.address);

            let account = Account::Basic(BasicAccount {
                balance: genesis_account.balance,
            });

            genesis_accounts.push((key, account));
        }

        debug!("Staking contract");
        // First generate the Staking contract in the Accounts.
        self.generate_staking_contract(&accounts, &mut txn)?;

        // Then get all the accounts from the Staking contract and add them to the genesis_accounts.
        // TODO: Maybe turn this code into a StakingContract method?
        genesis_accounts.push((
            StakingContract::get_key_staking_contract(),
            Account::Staking(StakingContract::get_staking_contract(&accounts.tree, &txn)),
        ));

        for validator in &self.validators {
            genesis_accounts.push((
                StakingContract::get_key_validator(&validator.validator_address),
                Account::StakingValidator(
                    StakingContract::get_validator(
                        &accounts.tree,
                        &txn,
                        &validator.validator_address,
                    )
                    .unwrap(),
                ),
            ));
        }

        for staker in &self.stakers {
            genesis_accounts.push((
                StakingContract::get_key_staker(&staker.staker_address),
                Account::StakingStaker(
                    StakingContract::get_staker(&accounts.tree, &txn, &staker.staker_address)
                        .unwrap(),
                ),
            ));

            genesis_accounts.push((
                StakingContract::get_key_validator_staker(
                    &staker.delegation,
                    &staker.staker_address,
                ),
                Account::StakingValidatorsStaker(staker.staker_address.clone()),
            ));
        }

        accounts.init(&mut txn, genesis_accounts.clone());

        // generate seeds
        // seed of genesis block = VRF(seed_0)
        let seed = self
            .vrf_seed
            .clone()
            .ok_or(GenesisBuilderError::NoVrfSeed)?;
        debug!("Genesis seed: {}", seed);

        // generate slot allocation from staking contract
        let slots = StakingContract::select_validators(&accounts.tree, &txn, &seed);
        debug!("Slots: {:#?}", slots);

        // Body
        let body = MacroBody {
            validators: Some(slots),
            ..Default::default()
        };

        let body_root = body.hash::<Blake2bHash>();
        debug!("Body root: {}", &body_root);

        // State root
        let state_root = accounts.get_root(Some(&txn));
        debug!("State root: {}", &state_root);
        txn.abort();

        // the header
        let header = MacroHeader {
            version: 1,
            block_number: 0,
            view_number: 0,
            timestamp: u64::try_from(timestamp.unix_timestamp())
                .map_err(|_| GenesisBuilderError::InvalidTimestamp(timestamp))?,
            parent_hash: [0u8; 32].into(),
            parent_election_hash: [0u8; 32].into(),
            seed,
            extra_data: vec![],
            state_root,
            body_root,
            history_root: Blake2bHash::default(),
        };

        // genesis hash
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
        txn: &mut WriteTransaction,
    ) -> Result<(), GenesisBuilderError> {
        StakingContract::create(&accounts.tree, txn);

        for validator in &self.validators {
            StakingContract::create_validator(
                &accounts.tree,
                txn,
                &validator.validator_address,
                validator.signing_key,
                validator.voting_key.compress(),
                validator.reward_address.clone(),
                None,
            )?;
        }

        for staker in &self.stakers {
            StakingContract::create_staker(
                &accounts.tree,
                txn,
                &staker.staker_address,
                staker.balance,
                Some(staker.delegation.clone()),
            )?;
        }

        Ok(())
    }

    pub fn write_to_files<P: AsRef<Path>>(
        &self,
        env: Environment,
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
        AccountsList(accounts).serialize(&mut file)?;

        Ok(hash)
    }
}

impl Default for GenesisBuilder {
    fn default() -> Self {
        Self::new()
    }
}
