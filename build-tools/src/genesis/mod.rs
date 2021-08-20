use std::convert::TryFrom;
use std::fs::{read_to_string, OpenOptions};
use std::io::Error as IoError;
use std::path::Path;

use chrono::{DateTime, Utc};
use thiserror::Error;
use toml::de::Error as TomlError;

use account::{Account, AccountError, Accounts, AccountsList, BasicAccount, StakingContract};
use beserial::{Deserialize, Serialize, SerializingError};
use block::{Block, MacroBlock, MacroBody, MacroHeader};
use bls::{PublicKey as BlsPublicKey, SecretKey as BlsSecretKey};
use database::volatile::{VolatileDatabaseError, VolatileEnvironment};
use database::WriteTransaction;
use hash::{Blake2bHash, Blake2sHasher, Hash, Hasher};
use keys::Address;
use nimiq_trie::key_nibbles::KeyNibbles;
use primitives::coin::Coin;
use vrf::VrfSeed;

mod config;

const DEFAULT_SIGNING_KEY: [u8; 96] = [0u8; 96];

#[derive(Debug, Error)]
pub enum GenesisBuilderError {
    #[error("No signing key to generate genesis seed.")]
    NoSigningKey,
    #[error("Invalid timestamp: {0}")]
    InvalidTimestamp(DateTime<Utc>),
    #[error("Serialization failed")]
    SerializingError(#[from] SerializingError),
    #[error("I/O error")]
    IoError(#[from] IoError),
    #[error("Failed to parse TOML file")]
    TomlError(#[from] TomlError),
    #[error("Failed to stake")]
    StakingError(#[from] AccountError),
    #[error("Database error")]
    DatabaseError(#[from] VolatileDatabaseError),
}

#[derive(Clone)]
pub struct GenesisInfo {
    pub block: Block,
    pub hash: Blake2bHash,
    pub accounts: Vec<(KeyNibbles, Account)>,
}

pub struct GenesisBuilder {
    pub signing_key: Option<BlsSecretKey>,
    pub seed_message: Option<String>,
    pub timestamp: Option<DateTime<Utc>>,
    pub validators: Vec<config::GenesisValidator>,
    pub stakers: Vec<config::GenesisStaker>,
    pub accounts: Vec<config::GenesisAccount>,
}

impl GenesisBuilder {
    pub fn new() -> Self {
        GenesisBuilder {
            signing_key: None,
            seed_message: None,
            timestamp: None,
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
        self.signing_key = Some(BlsSecretKey::deserialize_from_vec(&DEFAULT_SIGNING_KEY).unwrap());
        self
    }

    pub fn with_signing_key(&mut self, secret_key: BlsSecretKey) -> &mut Self {
        self.signing_key = Some(secret_key);
        self
    }

    pub fn with_seed_message<S: AsRef<str>>(&mut self, seed_message: S) -> &mut Self {
        self.seed_message = Some(seed_message.as_ref().to_string());
        self
    }

    pub fn with_timestamp(&mut self, timestamp: DateTime<Utc>) -> &mut Self {
        self.timestamp = Some(timestamp);
        self
    }

    pub fn with_genesis_validator(
        &mut self,
        validator_address: Address,
        warm_address: Address,
        validator_key: BlsPublicKey,
        reward_address: Address,
    ) -> &mut Self {
        self.validators.push(config::GenesisValidator {
            validator_address,
            warm_address,
            validator_key,
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
            signing_key,
            seed_message,
            timestamp,
            mut validators,
            mut stakers,
            mut accounts,
        } = toml::from_str(&read_to_string(path)?)?;

        signing_key.map(|skey| self.with_signing_key(skey));
        seed_message.map(|msg| self.with_seed_message(msg));
        timestamp.map(|t| self.with_timestamp(t));
        self.validators.append(&mut validators);
        self.stakers.append(&mut stakers);
        self.accounts.append(&mut accounts);

        Ok(self)
    }

    pub fn generate(&self) -> Result<GenesisInfo, GenesisBuilderError> {
        // Initialize the environment.
        let env = VolatileEnvironment::new(10)?;
        let timestamp = self.timestamp.unwrap_or_else(Utc::now);

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

        accounts.init(&mut txn, genesis_accounts.clone());

        debug!("Staking contract");
        // First generate the Staking contract in the Accounts.
        self.generate_staking_contract(&accounts, &mut txn)?;

        // Then get all the accounts from the Staking contract and add them to the genesis_accounts.
        // TODO: Maybe turn this code into a StakingContract method?
        genesis_accounts.push((
            StakingContract::get_key_staking_contract(),
            Account::Staking(StakingContract::get_staking_contract(
                &accounts.tree,
                &mut txn,
            )),
        ));

        for validator in &self.validators {
            genesis_accounts.push((
                StakingContract::get_key_validator(&validator.validator_address),
                Account::StakingValidator(
                    StakingContract::get_validator(
                        &accounts.tree,
                        &mut txn,
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
                    StakingContract::get_staker(&accounts.tree, &mut txn, &staker.staker_address)
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

        // generate seeds
        let signing_key = self
            .signing_key
            .as_ref()
            .ok_or(GenesisBuilderError::NoSigningKey)?;

        // random message used as seed for VRF that generates pre-genesis seed
        let seed_message = self.seed_message.clone().unwrap_or_else(|| {
            "love ai amor mohabbat hubun cinta lyubov bhalabasa amour kauna pi'ara liebe eshq upendo prema amore katresnan sarang anpu prema yeu".to_string()
        });

        // pre-genesis seed (used for slot selection)
        let pre_genesis_seed: VrfSeed = signing_key
            .sign_hash(Blake2sHasher::new().digest(seed_message.as_bytes()))
            .compress()
            .into();
        debug!("Pre genesis seed: {}", pre_genesis_seed);

        // seed of genesis block = VRF(seed_0)
        let seed = pre_genesis_seed.sign_next(signing_key);
        debug!("Genesis seed: {}", seed);

        // generate slot allocation from staking contract
        let slots = StakingContract::select_validators(&accounts.tree, &mut txn, &seed);
        debug!("Slots: {:#?}", slots);

        // Body
        let mut body = MacroBody::new();
        body.validators = Some(slots);

        let body_root = body.hash::<Blake2bHash>();
        debug!("Body root: {}", &body_root);

        // State root
        let state_root = accounts.get_root(Some(&txn));
        debug!("State root: {}", &state_root);

        // the header
        let header = MacroHeader {
            version: 1,
            block_number: 0,
            view_number: 0,
            timestamp: u64::try_from(timestamp.timestamp_millis())
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
                validator.warm_address.clone(),
                validator.validator_key.compress(),
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
        directory: P,
    ) -> Result<Blake2bHash, GenesisBuilderError> {
        let GenesisInfo {
            block,
            hash,
            accounts,
        } = self.generate()?;

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
