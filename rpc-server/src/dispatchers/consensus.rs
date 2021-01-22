use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::RwLock;

use beserial::{Deserialize, Serialize};
use nimiq_consensus_albatross::ConsensusProxy;
use nimiq_hash::{Blake2bHash, Hash};
use nimiq_mempool::ReturnCode;
use nimiq_network_libp2p::Network;
use nimiq_transaction::Transaction;
use nimiq_transaction_builder::TransactionBuilder;
use nimiq_keys::{Address, KeyPair};
use nimiq_primitives::{networks::NetworkId, coin::Coin};

use nimiq_rpc_interface::{
    consensus::ConsensusInterface,
    types::ValidityStartHeight,
};

use crate::{error::Error, wallets::UnlockedWallets};
use nimiq_bls::CompressedPublicKey;

pub struct ConsensusDispatcher {
    consensus: ConsensusProxy<Network>,

    unlocked_wallets: Option<Arc<RwLock<UnlockedWallets>>>,
}

impl ConsensusDispatcher {
    pub fn new(
        consensus: ConsensusProxy<Network>,
        unlocked_wallets: Option<Arc<RwLock<UnlockedWallets>>>,
    ) -> Self {
        Self {
            consensus,
            unlocked_wallets,
        }
    }

    async fn push_transaction(&self, tx: Transaction) -> Result<Blake2bHash, Error> {
        let txid = tx.hash::<Blake2bHash>();
        match self.consensus.send_transaction(tx).await {
            Ok(ReturnCode::Accepted) => Ok(txid),
            Ok(return_code) => Err(Error::TransactionRejected(return_code)),
            Err(e) => Err(Error::NetworkError(e)),
        }
    }

    fn get_wallet_keypair(&self, address: &Address) -> Result<KeyPair, Error> {
        Ok(self.unlocked_wallets
            .as_ref()
            .ok_or_else(|| Error::UnlockedWalletNotFound(address.clone()))?
            .read()
            .get(address)
            .ok_or_else(|| Error::UnlockedWalletNotFound(address.clone()))?
            .key_pair.clone()) // TODO: Avoid cloning
    }

    fn network_id(&self) -> NetworkId {
        self.consensus.blockchain.network_id
    }

    fn validity_start_height(&self, validity_start_height: ValidityStartHeight) -> u32 {
        validity_start_height.block_number(self.consensus.blockchain.block_number())
    }
}

fn transaction_to_hex_string(transaction: &Transaction) -> String {
    hex::encode(&transaction.serialize_to_vec())
}

#[nimiq_jsonrpc_derive::service(rename_all="camelCase")]
#[async_trait]
impl ConsensusInterface for ConsensusDispatcher {
    type Error = Error;

    async fn send_raw_transaction(&mut self, raw_tx: String) -> Result<Blake2bHash, Error> {
        let tx = Deserialize::deserialize_from_vec(&hex::decode(&raw_tx)?)?;
        self.push_transaction(tx).await
    }

    async fn create_basic_transaction(&mut self, wallet: Address, recipient: Address, value: Coin, fee: Coin, validity_start_height: ValidityStartHeight) -> Result<String, Error> {
        let transaction = TransactionBuilder::new_simple(
            &self.get_wallet_keypair(&wallet)?,
            recipient,
            value,
            fee,
            self.validity_start_height(validity_start_height),
            self.network_id(),
        );

        Ok(transaction_to_hex_string(&transaction))
    }

    async fn send_basic_transaction(&mut self, wallet: Address, recipient: Address, value: Coin, fee: Coin, validity_start_height: ValidityStartHeight) -> Result<Blake2bHash, Error> {
        let transaction = TransactionBuilder::new_simple(
            &self.get_wallet_keypair(&wallet)?,
            recipient,
            value,
            fee,
            self.validity_start_height(validity_start_height),
            self.network_id(),
        );

        self.push_transaction(transaction).await
    }

    async fn create_stake_transaction(&mut self, wallet: Address, validator_key: CompressedPublicKey, value: Coin, fee: Coin, validity_start_height: ValidityStartHeight) -> Result<String, Error> {
        let transaction = TransactionBuilder::new_stake(
            None,
            &self.get_wallet_keypair(&wallet)?,
            &validator_key.uncompress()?,
            value,
            fee,
            self.validity_start_height(validity_start_height),
            self.network_id(),
        );

        Ok(transaction_to_hex_string(&transaction))
    }

    async fn send_stake_transaction(&mut self, wallet: Address, validator_key: CompressedPublicKey, value: Coin, fee: Coin, validity_start_height: ValidityStartHeight) -> Result<Blake2bHash, Error> {
        let transaction = TransactionBuilder::new_stake(
            None,
            &self.get_wallet_keypair(&wallet)?,
            &validator_key.uncompress()?,
            value,
            fee,
            self.validity_start_height(validity_start_height),
            self.network_id(),
        );

        self.push_transaction(transaction).await
    }

    async fn create_retire_transaction(&mut self, wallet: Address, validator_key: CompressedPublicKey, value: Coin, fee: Coin, validity_start_height: ValidityStartHeight) -> Result<String, Error> {
        let transaction = TransactionBuilder::new_retire(
            None,
            &self.get_wallet_keypair(&wallet)?,
            &validator_key.uncompress()?,
            value,
            fee,
            self.validity_start_height(validity_start_height),
            self.network_id(),
        );

        Ok(transaction_to_hex_string(&transaction))
    }

    async fn send_retire_transaction(&mut self, wallet: Address, validator_key: CompressedPublicKey, value: Coin, fee: Coin, validity_start_height: ValidityStartHeight) -> Result<Blake2bHash, Error> {
        let transaction = TransactionBuilder::new_retire(
            None,
            &self.get_wallet_keypair(&wallet)?,
            &validator_key.uncompress()?,
            value,
            fee,
            self.validity_start_height(validity_start_height),
            self.network_id(),
        );

        self.push_transaction(transaction).await
    }

    async fn create_reactivate_transaction(&mut self, wallet: Address, validator_key: CompressedPublicKey, value: Coin, fee: Coin, validity_start_height: ValidityStartHeight) -> Result<String, Error> {
        let transaction = TransactionBuilder::new_reactivate(
            None,
            &self.get_wallet_keypair(&wallet)?,
            &validator_key.uncompress()?,
            value,
            fee,
            self.validity_start_height(validity_start_height),
            self.network_id(),
        );

        Ok(transaction_to_hex_string(&transaction))
    }

    async fn send_reactivate_transaction(&mut self, wallet: Address, validator_key: CompressedPublicKey, value: Coin, fee: Coin, validity_start_height: ValidityStartHeight) -> Result<Blake2bHash, Error> {
        let transaction = TransactionBuilder::new_reactivate(
            None,
            &self.get_wallet_keypair(&wallet)?,
            &validator_key.uncompress()?,
            value,
            fee,
            self.validity_start_height(validity_start_height),
            self.network_id(),
        );

        self.push_transaction(transaction).await
    }

    async fn create_unstake_transaction(&mut self, wallet: Address, recipient: Address, value: Coin, fee: Coin, validity_start_height: ValidityStartHeight) -> Result<String, Error> {
        let transaction = TransactionBuilder::new_unstake(
            None,
            &self.get_wallet_keypair(&wallet)?,
            recipient,
            value,
            fee,
            self.validity_start_height(validity_start_height),
            self.network_id(),
        );

        Ok(transaction_to_hex_string(&transaction))
    }

    async fn send_unstake_transaction(&mut self, wallet: Address, recipient: Address, value: Coin, fee: Coin, validity_start_height: ValidityStartHeight) -> Result<Blake2bHash, Error> {
        let transaction = TransactionBuilder::new_unstake(
            None,
            &self.get_wallet_keypair(&wallet)?,
            recipient,
            value,
            fee,
            self.validity_start_height(validity_start_height),
            self.network_id(),
        );

        self.push_transaction(transaction).await
    }
}
