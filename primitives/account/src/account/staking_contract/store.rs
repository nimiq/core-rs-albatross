use nimiq_keys::Address;
#[cfg(feature = "interaction-traits")]
use nimiq_primitives::account::AccountError;
use nimiq_primitives::key_nibbles::KeyNibbles;

#[cfg(feature = "interaction-traits")]
use crate::data_store::DataStoreWrite;
use crate::{
    account::staking_contract::{validator::Tombstone, Staker, Validator},
    data_store_ops::{DataStoreIterOps, DataStoreReadOps},
};

// Fixme: This shouldn't be pub but for now it is needed for `RemoteDataStore`
pub struct StakingContractStore {}

impl StakingContractStore {
    const PREFIX_VALIDATOR: u8 = 0;
    const PREFIX_STAKER: u8 = 1;
    const PREFIX_TOMBSTONE: u8 = 2;

    pub fn validator_key(address: &Address) -> KeyNibbles {
        Self::prefixed_address(Self::PREFIX_VALIDATOR, address)
    }

    pub fn staker_key(address: &Address) -> KeyNibbles {
        Self::prefixed_address(Self::PREFIX_STAKER, address)
    }

    pub fn tombstone_key(address: &Address) -> KeyNibbles {
        Self::prefixed_address(Self::PREFIX_TOMBSTONE, address)
    }

    fn prefixed_address(prefix: u8, address: &Address) -> KeyNibbles {
        let mut key = [0u8; 21];
        key[0] = prefix;
        key[1..].copy_from_slice(&address.0);
        KeyNibbles::from(&key[..])
    }
}

pub trait StakingContractStoreReadOps {
    fn get_validator(&self, address: &Address) -> Option<Validator>;

    fn get_staker(&self, address: &Address) -> Option<Staker>;

    fn get_tombstone(&self, address: &Address) -> Option<Tombstone>;
}

pub(crate) struct StakingContractStoreRead<'read, T: DataStoreReadOps>(&'read T);

impl<'read, T: DataStoreReadOps> StakingContractStoreRead<'read, T> {
    pub fn new(data_store: &'read T) -> Self {
        StakingContractStoreRead(data_store)
    }
}

impl<'read, T: DataStoreReadOps> StakingContractStoreReadOps
    for StakingContractStoreRead<'read, T>
{
    fn get_validator(&self, address: &Address) -> Option<Validator> {
        self.0.get(&StakingContractStore::validator_key(address))
    }

    fn get_staker(&self, address: &Address) -> Option<Staker> {
        self.0.get(&StakingContractStore::staker_key(address))
    }

    fn get_tombstone(&self, address: &Address) -> Option<Tombstone> {
        self.0.get(&StakingContractStore::tombstone_key(address))
    }
}

impl<'read, T: DataStoreReadOps + DataStoreIterOps> StakingContractStoreRead<'read, T> {
    pub(crate) fn iter_stakers(&self) -> impl Iterator<Item = Staker> {
        self.0.iter(
            &StakingContractStore::staker_key(&Address::START_ADDRESS),
            &StakingContractStore::staker_key(&Address::END_ADDRESS),
        )
    }

    pub(crate) fn iter_validators(&self) -> impl Iterator<Item = Validator> {
        self.0.iter(
            &StakingContractStore::validator_key(&Address::START_ADDRESS),
            &StakingContractStore::validator_key(&Address::END_ADDRESS),
        )
    }
}

#[cfg(feature = "interaction-traits")]
pub struct StakingContractStoreWrite<'write, 'store, 'tree, 'txn, 'txni, 'env>(
    &'write mut DataStoreWrite<'store, 'tree, 'txn, 'txni, 'env>,
);

#[cfg(feature = "interaction-traits")]
impl<'write, 'store, 'tree, 'txn, 'txni, 'env>
    StakingContractStoreWrite<'write, 'store, 'tree, 'txn, 'txni, 'env>
{
    pub fn new(data_store: &'write mut DataStoreWrite<'store, 'tree, 'txn, 'txni, 'env>) -> Self {
        StakingContractStoreWrite(data_store)
    }

    pub fn put_validator(&mut self, address: &Address, validator: Validator) {
        self.0
            .put(&StakingContractStore::validator_key(address), validator)
    }

    pub fn remove_validator(&mut self, address: &Address) {
        self.0.remove(&StakingContractStore::validator_key(address))
    }

    pub fn put_staker(&mut self, address: &Address, staker: Staker) {
        self.0
            .put(&StakingContractStore::staker_key(address), staker)
    }

    pub fn remove_staker(&mut self, address: &Address) {
        self.0.remove(&StakingContractStore::staker_key(address))
    }

    pub fn put_tombstone(&mut self, address: &Address, tombstone: Tombstone) {
        self.0
            .put(&StakingContractStore::tombstone_key(address), tombstone)
    }

    pub fn remove_tombstone(&mut self, address: &Address) {
        self.0.remove(&StakingContractStore::tombstone_key(address))
    }
}

#[cfg(feature = "interaction-traits")]
impl<'write, 'store, 'tree, 'txn, 'txni, 'env> StakingContractStoreReadOps
    for StakingContractStoreWrite<'write, 'store, 'tree, 'txn, 'txni, 'env>
{
    fn get_validator(&self, address: &Address) -> Option<Validator> {
        self.0.get(&StakingContractStore::validator_key(address))
    }

    fn get_staker(&self, address: &Address) -> Option<Staker> {
        self.0.get(&StakingContractStore::staker_key(address))
    }

    fn get_tombstone(&self, address: &Address) -> Option<Tombstone> {
        self.0.get(&StakingContractStore::tombstone_key(address))
    }
}

#[cfg(feature = "interaction-traits")]
pub trait StakingContractStoreReadOpsExt {
    fn expect_validator(&self, address: &Address) -> Result<Validator, AccountError>;

    fn expect_staker(&self, address: &Address) -> Result<Staker, AccountError>;
}

#[cfg(feature = "interaction-traits")]
impl<T: StakingContractStoreReadOps> StakingContractStoreReadOpsExt for T {
    fn expect_validator(&self, address: &Address) -> Result<Validator, AccountError> {
        self.get_validator(address)
            .ok_or_else(|| AccountError::NonExistentAddress {
                address: address.clone(),
            })
    }

    fn expect_staker(&self, address: &Address) -> Result<Staker, AccountError> {
        self.get_staker(address)
            .ok_or_else(|| AccountError::NonExistentAddress {
                address: address.clone(),
            })
    }
}
