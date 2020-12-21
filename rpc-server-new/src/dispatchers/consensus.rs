use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::RwLock;

use beserial::{Deserialize, Serialize};
use nimiq_hash::Blake2bHash;
use nimiq_consensus_albatross::Consensus;
use nimiq_transaction::Transaction;
use nimiq_hash::Hash;
use nimiq_primitives::account::AccountType;
use nimiq_network_libp2p::Network;

use crate::{
    types::TransactionParameters,
    wallets::UnlockedWallets,
    Error,
};


#[async_trait]
pub trait ConsensusInterface {
    async fn send_raw_transaction(&self, raw_tx: String) -> Result<String, Error>;

    async fn create_raw_transaction(&self, tx_params: TransactionParameters) -> Result<String, Error>;

    async fn send_transaction(&self, tx_params: TransactionParameters) -> Result<String, Error>;
}


pub struct ConsensusDispatcher {
    consensus: Arc<Consensus<Network>>,

    unlocked_wallets: Option<Arc<RwLock<UnlockedWallets>>>,
}

impl ConsensusDispatcher {
    pub fn new(consensus: Arc<Consensus<Network>>, unlocked_wallets: Option<Arc<RwLock<UnlockedWallets>>>) -> Self {
        Self {
            consensus,
            unlocked_wallets,
        }
    }

    fn push_transaction(&self, tx: Transaction) -> Result<Blake2bHash, Error> {
        let txid = tx.hash::<Blake2bHash>();
        /*match self.mempool.push_transaction(tx) {
            ReturnCode::Accepted | ReturnCode::Known => Ok(txid),
            code => Err(Error::TransactionRejected(code)),
        }*/
        log::error!("TODO: push_transaction will be moved to the Consensus: {:#?}", tx);
        Ok(txid)
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

#[nimiq_jsonrpc_derive::service(rename_all = "mixedCase")]
#[async_trait]
impl ConsensusInterface for ConsensusDispatcher {
    async fn send_raw_transaction(&self, raw_tx: String) -> Result<String, Error> {
        let tx = Deserialize::deserialize_from_vec(&hex::decode(&raw_tx)?)?;
        Ok(self.push_transaction(tx)?.to_hex())
    }

    async fn create_raw_transaction(&self, tx_params: TransactionParameters) -> Result<String, Error> {
        let mut tx = tx_params.into_transaction(&self.consensus.blockchain)?;

        if tx.sender_type == AccountType::Basic {
            self.sign_transaction(&mut tx)?;
        }

        Ok(hex::encode(&tx.serialize_to_vec()))
    }

    async fn send_transaction(&self, tx_params: TransactionParameters) -> Result<String, Error> {
        let mut tx = tx_params.into_transaction(&self.consensus.blockchain)?;

        if tx.sender_type == AccountType::Basic {
            self.sign_transaction(&mut tx)?;
        }

        Ok(self.push_transaction(tx)?.to_hex())
    }
}