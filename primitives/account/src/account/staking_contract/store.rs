use nimiq_keys::Address;
use nimiq_trie::key_nibbles::KeyNibbles;

use crate::account::staking_contract::validator::Tombstone;
use crate::data_store::{DataStoreRead, DataStoreWrite};
use crate::{AccountError, Staker, Validator};

struct StakingContractStore {}

impl StakingContractStore {
    const PREFIX_VALIDATOR: u8 = 0;
    const PREFIX_STAKER: u8 = 1;
    const PREFIX_TOMBSTONE: u8 = 2;

    fn validator_key(address: &Address) -> KeyNibbles {
        Self::prefixed_address(Self::PREFIX_VALIDATOR, address)
    }

    fn staker_key(address: &Address) -> KeyNibbles {
        Self::prefixed_address(Self::PREFIX_STAKER, address)
    }

    fn tombstone_key(address: &Address) -> KeyNibbles {
        Self::prefixed_address(Self::PREFIX_TOMBSTONE, address)
    }

    fn prefixed_address(prefix: u8, address: &Address) -> KeyNibbles {
        let mut key = Vec::with_capacity(21);
        key.push(prefix);
        key.extend(address.as_slice());
        KeyNibbles::from(key)
    }
}

pub trait StakingContractStoreReadOps {
    fn get_validator(&self, address: &Address) -> Option<Validator>;

    fn get_staker(&self, address: &Address) -> Option<Staker>;

    fn get_tombstone(&self, address: &Address) -> Option<Tombstone>;
}

pub(crate) struct StakingContractStoreRead<'read, 'store, 'tree, 'txn, 'env>(
    &'read DataStoreRead<'store, 'tree, 'txn, 'env>,
);

impl<'read, 'store, 'tree, 'txn, 'env> StakingContractStoreRead<'read, 'store, 'tree, 'txn, 'env> {
    pub fn new(data_store: &'read DataStoreRead) -> Self {
        StakingContractStoreRead(data_store)
    }
}

impl<'read, 'store, 'tree, 'txn, 'env> StakingContractStoreReadOps
    for StakingContractStoreRead<'read, 'store, 'tree, 'txn, 'env>
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

pub(crate) struct StakingContractStoreWrite<'write, 'store, 'tree, 'txn, 'env>(
    &'write mut DataStoreWrite<'store, 'tree, 'txn, 'env>,
);

impl<'write, 'store, 'tree, 'txn, 'env>
    StakingContractStoreWrite<'write, 'store, 'tree, 'txn, 'env>
{
    pub fn new(data_store: &'write mut DataStoreWrite) -> Self {
        StakingContractStoreWrite(data_store)
    }

    pub fn put_validator(&self, address: &Address, validator: Validator) {
        self.0
            .put(&StakingContractStore::validator_key(address), validator)
    }

    pub fn remove_validator(self, address: &Address) {
        self.0.remove(&StakingContractStore::validator_key(address))
    }

    pub fn put_staker(&self, address: &Address, staker: Staker) {
        self.0
            .put(&StakingContractStore::staker_key(address), staker)
    }

    pub fn remove_staker(self, address: &Address) {
        self.0.remove(&StakingContractStore::staker_key(address))
    }

    pub fn put_tombstone(&self, address: &Address, tombstone: Tombstone) {
        self.0
            .put(&StakingContractStore::tombstone_key(address), tombstone)
    }

    pub fn remove_tombstone(self, address: &Address) {
        self.0.remove(&StakingContractStore::tombstone_key(address))
    }
}

impl<'write, 'store, 'tree, 'txn, 'env> StakingContractStoreReadOps
    for StakingContractStoreWrite<'write, 'store, 'tree, 'txn, 'env>
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

pub trait StakingContractStoreReadOpsExt {
    fn expect_validator(&self, address: &Address) -> Result<Validator, AccountError>;

    fn expect_staker(&self, address: &Address) -> Result<Staker, AccountError>;
}

impl<T: StakingContractStoreReadOps> StakingContractStoreReadOpsExt for T {
    fn expect_validator(&self, address: &Address) -> Result<Validator, AccountError> {
        self.get_validator(address)
            .ok_or_else(|| AccountError::NonExistentAddress(address.clone()))
    }

    fn expect_staker(&self, address: &Address) -> Result<Staker, AccountError> {
        self.get_staker(address)
            .ok_or_else(|| AccountError::NonExistentAddress(address.clone()))
    }
}
