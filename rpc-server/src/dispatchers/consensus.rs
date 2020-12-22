use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::RwLock;

use beserial::{Deserialize, Serialize};
use nimiq_consensus_albatross::ConsensusProxy;
use nimiq_hash::Blake2bHash;
use nimiq_hash::Hash;
use nimiq_mempool::ReturnCode;
use nimiq_network_libp2p::Network;
use nimiq_primitives::account::AccountType;
use nimiq_rpc_interface::{consensus::ConsensusInterface, types::TransactionParameters};
use nimiq_transaction::Transaction;

use crate::{error::Error, wallets::UnlockedWallets};

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

    fn sign_transaction(&self, tx: &mut Transaction) -> Result<(), Error> {
        self.unlocked_wallets
            .as_ref()
            .ok_or_else(|| Error::UnlockedWalletNotFound(tx.sender.clone()))?
            .read()
            .get(&tx.sender)
            .ok_or_else(|| Error::UnlockedWalletNotFound(tx.sender.clone()))?
            .sign_transaction(tx);
        Ok(())
    }
}

#[nimiq_jsonrpc_derive::service]
#[async_trait]
impl ConsensusInterface for ConsensusDispatcher {
    type Error = Error;

    async fn send_raw_transaction(&mut self, raw_tx: String) -> Result<String, Error> {
        let tx = Deserialize::deserialize_from_vec(&hex::decode(&raw_tx)?)?;
        Ok(self.push_transaction(tx).await?.to_hex())
    }

    async fn create_raw_transaction(
        &mut self,
        tx_params: TransactionParameters,
    ) -> Result<String, Error> {
        let mut tx = tx_params.into_transaction(&self.consensus.blockchain)?;

        if tx.sender_type == AccountType::Basic {
            self.sign_transaction(&mut tx)?;
        }

        Ok(hex::encode(&tx.serialize_to_vec()))
    }

    async fn send_transaction(
        &mut self,
        tx_params: TransactionParameters,
    ) -> Result<String, Error> {
        let mut tx = tx_params.into_transaction(&self.consensus.blockchain)?;

        if tx.sender_type == AccountType::Basic {
            self.sign_transaction(&mut tx)?;
        }

        Ok(self.push_transaction(tx).await?.to_hex())
    }
}
