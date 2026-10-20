use nimiq_keys::Address;
use nimiq_primitives::key_nibbles::KeyNibbles;
use nimiq_serde::{Deserialize, Serialize};

#[cfg(feature = "interaction-traits")]
use crate::data_store::DataStoreWrite;

/// Represents a nonce entry for an address in the bridge contract
#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct BridgeNonce {
    /// The highest processed nonce for this address
    pub nonce: u64,
}

// Fixme: This shouldn't be pub but for now it is needed for `RemoteDataStore`
pub struct BridgeContractStore {}

impl BridgeContractStore {
    const PREFIX_NONCE: u8 = 0;

    pub fn nonce_key(address: &Address) -> KeyNibbles {
        Self::prefixed_address(Self::PREFIX_NONCE, address)
    }

    fn prefixed_address(prefix: u8, address: &Address) -> KeyNibbles {
        let mut key = [0u8; 21];
        key[0] = prefix;
        key[1..].copy_from_slice(&address.0);
        KeyNibbles::from(&key[..])
    }
}

#[cfg(feature = "interaction-traits")]
pub struct BridgeContractStoreWrite<'write, 'store, 'tree, 'txn, 'txni, 'env>(
    &'write mut DataStoreWrite<'store, 'tree, 'txn, 'txni, 'env>,
);

#[cfg(feature = "interaction-traits")]
impl<'write, 'store, 'tree, 'txn, 'txni, 'env>
    BridgeContractStoreWrite<'write, 'store, 'tree, 'txn, 'txni, 'env>
{
    pub fn new(data_store: &'write mut DataStoreWrite<'store, 'tree, 'txn, 'txni, 'env>) -> Self {
        BridgeContractStoreWrite(data_store)
    }

    pub fn put_nonce(&mut self, address: &Address, nonce: BridgeNonce) {
        self.0.put(&BridgeContractStore::nonce_key(address), nonce)
    }

    pub fn remove_nonce(&mut self, address: &Address) {
        self.0.remove(&BridgeContractStore::nonce_key(address))
    }
}
