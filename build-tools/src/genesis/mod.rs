use std::convert::TryFrom;
use std::fs::{read_to_string, OpenOptions};
use std::io::Error as IoError;
use std::path::Path;

use chrono::{DateTime, Utc};
use thiserror::Error;
use toml::de::Error as TomlError;

use account::{Account, AccountError, AccountsList, BasicAccount, StakingContract};
use accounts::Accounts;
use beserial::{Deserialize, Serialize, SerializingError};
use block::{Block, MacroBlock, MacroBody, MacroHeader};
use bls::{PublicKey as BlsPublicKey, SecretKey as BlsSecretKey};
use database::volatile::{VolatileDatabaseError, VolatileEnvironment};
use database::WriteTransaction;
use hash::{Blake2bHash, Blake2sHasher, Hash, Hasher};
use keys::Address;
use primitives::account::ValidatorId;
use primitives::coin::Coin;
use vrf::VrfSeed;

mod config;

const DEFAULT_SIGNING_KEY: [u8; 96] = [0u8; 96];
const DEFAULT_STAKING_CONTRACT_ADDRESS: &str = "NQ38 STAK 1NG0 0000 0000 C0NT RACT 0000 0000";

#[derive(Debug, Error)]
pub enum GenesisBuilderError {
    #[error("No signing key to generate genesis seed.")]
    NoSigningKey,
    #[error("No staking contract address.")]
    NoStakingContractAddress,
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
    pub accounts: Vec<(Address, Account)>,
}

pub struct GenesisBuilder {
    pub signing_key: Option<BlsSecretKey>,
    pub seed_message: Option<String>,
    pub timestamp: Option<DateTime<Utc>>,
    pub validators: Vec<config::GenesisValidator>,
    pub stakes: Vec<config::GenesisStake>,
    pub accounts: Vec<config::GenesisAccount>,
    pub staking_contract_address: Option<Address>,
}

impl GenesisBuilder {
    pub fn new() -> Self {
        GenesisBuilder {
            signing_key: None,
            seed_message: None,
            timestamp: None,
            validators: vec![],
            stakes: vec![],
            accounts: vec![],
            staking_contract_address: None,
        }
    }

    pub fn default() -> Self {
        let mut builder = Self::new();
        builder.with_defaults();
        builder
    }

    pub fn with_defaults(&mut self) -> &mut Self {
        self.signing_key = Some(BlsSecretKey::deserialize_from_vec(&DEFAULT_SIGNING_KEY).unwrap());
        self.staking_contract_address =
            Some(Address::from_user_friendly_address(DEFAULT_STAKING_CONTRACT_ADDRESS).unwrap());
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

    pub fn with_staking_contract_address(&mut self, address: Address) -> &mut Self {
        self.staking_contract_address = Some(address);
        self
    }

    pub fn with_genesis_validator(
        &mut self,
        validator_id: ValidatorId,
        validator_key: BlsPublicKey,
        reward_address: Address,
        balance: Coin,
    ) -> &mut Self {
        self.validators.push(config::GenesisValidator {
            validator_id,
            reward_address,
            balance,
            validator_key,
        });
        self
    }

    pub fn with_genesis_stake(
        &mut self,
        staker_address: Address,
        validator_id: ValidatorId,
        balance: Coin,
    ) -> &mut Self {
        self.stakes.push(config::GenesisStake {
            staker_address,
            balance,
            validator_id,
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
            mut stakes,
            mut accounts,
            staking_contract,
        } = toml::from_str(&read_to_string(path)?)?;

        signing_key.map(|skey| self.with_signing_key(skey));
        seed_message.map(|msg| self.with_seed_message(msg));
        timestamp.map(|t| self.with_timestamp(t));
        staking_contract.map(|address| self.with_staking_contract_address(address));
        self.validators.append(&mut validators);
        self.stakes.append(&mut stakes);
        self.accounts.append(&mut accounts);

        Ok(self)
    }

    pub fn generate(&self) -> Result<GenesisInfo, GenesisBuilderError> {
        let timestamp = self.timestamp.unwrap_or_else(Utc::now);

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

        // generate staking contract
        let staking_contract = self.generate_staking_contract()?;
        debug!("Staking contract: {:#?}", staking_contract);

        // generate slot allocation from staking contract
        let slots = staking_contract.select_validators(&seed);
        debug!("Slots: {:#?}", slots);

        // Body
        let mut body = MacroBody::new();
        body.validators = Some(slots);
        let body_root = body.hash::<Blake2bHash>();
        debug!("Body root: {}", &body_root);

        // accounts
        let mut genesis_accounts: Vec<(Address, Account)> = Vec::new();
        genesis_accounts.push((
            Address::clone(
                self.staking_contract_address
                    .as_ref()
                    .ok_or(GenesisBuilderError::NoStakingContractAddress)?,
            ),
            Account::Staking(staking_contract),
        ));
        for genesis_account in &self.accounts {
            let address = genesis_account.address.clone();
            let account = Account::Basic(BasicAccount {
                balance: genesis_account.balance,
            });
            debug!("Adding genesis account: {}: {:?}", address, account);
            genesis_accounts.push((address, account));
        }

        // state root
        let state_root = {
            let env = VolatileEnvironment::new(10)?;
            let accounts = Accounts::new(env.clone());
            let mut txn = WriteTransaction::new(&env);
            // XXX need to clone, since init needs the actual data
            accounts.init(&mut txn, genesis_accounts.clone());
            accounts.hash(Some(&txn))
        };
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

    fn generate_staking_contract(&self) -> Result<StakingContract, GenesisBuilderError> {
        let mut contract = StakingContract::default();

        for validator in self.validators.iter() {
            contract.create_validator(
                validator.validator_id.clone(),
                validator.validator_key.compress(),
                validator.reward_address.clone(),
                validator.balance,
            )?;
        }

        for stake in self.stakes.iter() {
            contract.create_staker(
                stake.staker_address.clone(),
                stake.balance,
                &stake.validator_id,
            )?;
        }

        Ok(contract)
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
