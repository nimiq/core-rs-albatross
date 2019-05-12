mod config;

use chrono::{DateTime, Utc};
use hash::{Blake2bHasher, Hasher, Blake2bHash, Hash};
use keys::Address;
use bls::bls12_381::{
    PublicKey as BlsPublicKey,
    SecretKey as BlsSecretKey,
};
use block_albatross::{Block, MacroBlock, MacroHeader};
use std::path::Path;
use beserial::{Serialize, SerializingError};
use std::fs::{OpenOptions, read_to_string};
use std::io::Error as IoError;
use toml::de::Error as TomlError;
use failure::Fail;
use primitives::coin::Coin;
use account::{Account, BasicAccount, StakingContract, AccountError, STAKING_CONTRACT_ADDRESS, AccountsList};
use std::convert::TryFrom;


#[derive(Debug, Fail)]
pub enum GenesisBuilderError {
    #[fail(display = "No signing key to generate genesis seed.")]
    NoSigningKey,
    #[fail(display = "Invalid timestamp: {}", _0)]
    InvalidTimestamp(DateTime<Utc>),
    #[fail(display = "Serialization failed")]
    SerializingError(#[cause] SerializingError),
    #[fail(display = "I/O error")]
    IoError(#[cause] IoError),
    #[fail(display = "Failed to parse TOML file")]
    TomlError(#[cause] TomlError),
    #[fail(display = "Failed to stake")]
    StakingError(#[cause] AccountError),
}

impl From<SerializingError> for GenesisBuilderError {
    fn from(e: SerializingError) -> Self {
        GenesisBuilderError::SerializingError(e)
    }
}

impl From<IoError> for GenesisBuilderError {
    fn from(e: IoError) -> Self {
        GenesisBuilderError::IoError(e)
    }
}

impl From<TomlError> for GenesisBuilderError {
    fn from(e: TomlError) -> Self {
        GenesisBuilderError::TomlError(e)
    }
}

impl From<AccountError> for GenesisBuilderError {
    fn from(e: AccountError) -> Self {
        GenesisBuilderError::StakingError(e)
    }
}


#[derive(Default)]
pub struct GenesisBuilder {
    signing_key: Option<BlsSecretKey>,
    seed_message: Option<String>,
    timestamp: Option<DateTime<Utc>>,
    stakes: Vec<config::GenesisStake>,
    accounts: Vec<config::GenesisAccount>,
}

impl GenesisBuilder {
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

    pub fn with_genesis_stake(&mut self, staker_address: Address, reward_address: Option<Address>, validator_key: BlsPublicKey, balance: Coin) -> &mut Self {
        self.stakes.push(config::GenesisStake {
            staker_address,
            reward_address,
            validator_key,
            balance
        });
        self
    }

    pub fn with_basic_account(&mut self, address: Address, balance: Coin) -> &mut Self {
        self.accounts.push(config::GenesisAccount {
            address,
            balance
        });
        self
    }

    pub fn with_config_file<P: AsRef<Path>>(&mut self, path: P) -> Result<&mut Self, GenesisBuilderError> {
        let config::GenesisConfig {
            signing_key,
            seed_message,
            timestamp,
            mut stakes,
            mut accounts,
        } = toml::from_str(&read_to_string(path)?)?;

        signing_key.map(|skey| self.with_signing_key(skey));
        seed_message.map(|msg| self.with_seed_message(msg));
        timestamp.map(|t| self.with_timestamp(t));
        self.stakes.append(&mut stakes);
        self.accounts.append(&mut accounts);

        Ok(self)
    }

    pub fn generate_header(&self) -> Result<MacroHeader, GenesisBuilderError> {
        let timestamp = self.timestamp.unwrap_or_else(Utc::now);
        let signing_key = self.signing_key.as_ref().ok_or(GenesisBuilderError::NoSigningKey)?;
        let seed_message = self.seed_message.clone()
            .unwrap_or("love ai amor mohabbat hubun cinta lyubov bhalabasa amour kauna pi'ara liebe eshq upendo prema amore katresnan sarang anpu prema yeu".to_string());
        let seed_hash = Blake2bHasher::new().digest(seed_message.as_bytes());

        Ok(MacroHeader {
            version: 1,
            validators: vec![], // TODO
            block_number: 1,
            view_number: 0,
            parent_macro_hash: [0u8; 32].into(),
            seed: signing_key.sign_hash(seed_hash),
            parent_hash: [0u8; 32].into(),
            state_root: [0u8; 32].into(), // TODO
            extrinsics_root: [0u8; 32].into(), // TODO
            timestamp: u64::try_from(timestamp.timestamp_millis())
                .map_err(|_| GenesisBuilderError::InvalidTimestamp(timestamp))?,
        })
    }

    pub fn generate_genesis_hash(&self) -> Result<Blake2bHash, GenesisBuilderError> {
        Ok(self.generate_header()?.hash::<Blake2bHash>())
    }

    pub fn generate_block(&self) -> Result<Block, GenesisBuilderError> {
        Ok(Block::Macro(MacroBlock {
            header: self.generate_header()?,
            justification: None,
            extrinsics: None // TODO
        }))
    }

    pub fn generate_accounts(&self) -> Result<Vec<(Address, Account)>, GenesisBuilderError> {
        let mut accounts: Vec<(Address, Account)> = Vec::new();
        accounts.push((Address::clone(&STAKING_CONTRACT_ADDRESS), Account::Staking(self.generate_staking_contract()?)));
        for account in &self.accounts {
            accounts.push((account.address.clone(), Account::Basic(BasicAccount { balance: account.balance })));
        }
        Ok(accounts)
    }

    pub fn generate_staking_contract(&self) -> Result<StakingContract, GenesisBuilderError> {
        let mut contract = StakingContract::default();

        for stake in self.stakes.iter() {
            contract.stake(&stake.staker_address, stake.balance, stake.validator_key.clone(), stake.reward_address.clone())?;
        }

        Ok(contract)
    }

    pub fn write_to_files<P: AsRef<Path>>(&self, directory: P) -> Result<(), GenesisBuilderError> {
        let block_path = directory.as_ref().join("block.dat");
        info!("Writing block to {}", block_path.display());
        let mut file = OpenOptions::new().create(true).write(true).open(&block_path)?;
        self.generate_block()?.serialize(&mut file)?;

        let accounts_path = directory.as_ref().join("accounts.dat");
        info!("Writing accounts to {}", accounts_path.display());
        let mut file = OpenOptions::new().create(true).write(true).open(&accounts_path)?;
        AccountsList(self.generate_accounts()?).serialize(&mut file)?;

        Ok(())
    }
}
