use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::RwLock;

use nimiq_hash::Blake2bHash;
use nimiq_mempool::Mempool;

use crate::{wallets::UnlockedWallets, Error};

#[async_trait]
pub trait MempoolInterface {
    async fn get_transaction(&self, txid: Blake2bHash) -> Result<Option<()>, Error>;

    async fn mempool_content(&self, include_transactions: bool) -> Result<Vec<()>, Error>;

    async fn mempool(&self) -> Result<(), Error>;

    async fn get_mempool_transaction(&self) -> Result<(), Error>;
}

pub struct MempoolDispatcher {
    mempool: Arc<Mempool>,

    //#[cfg(feature = "validator")]
    //validator: Option<Arc<Validator>>,
    unlocked_wallets: Option<Arc<RwLock<UnlockedWallets>>>,
}

impl MempoolDispatcher {
    pub fn new(mempool: Arc<Mempool>) -> Self {
        MempoolDispatcher {
            mempool,

            //#[cfg(feature = "validator")]
            //validator: None,
            unlocked_wallets: None,
        }
    }


}

#[nimiq_jsonrpc_derive::service(rename_all = "mixedCase")]
#[async_trait]
impl MempoolInterface for MempoolDispatcher {


    async fn get_transaction(&self, _txid: Blake2bHash) -> Result<Option<()>, Error> {
        /*Ok(self.mempool
        .get_transaction(&txid)
        // TODO: We can return a `Arc<Transaction>`, if we implement `serde::Serialize` for it.
        .map(|tx| Transaction::clone(&tx)))*/
        Err(Error::NotImplemented)
    }

    async fn mempool_content(&self, _include_transactions: bool) -> Result<Vec<()>, Error> {
        Err(Error::NotImplemented)
    }

    async fn mempool(&self) -> Result<(), Error> {
        Err(Error::NotImplemented)
    }

    async fn get_mempool_transaction(&self) -> Result<(), Error> {
        Err(Error::NotImplemented)
    }
}
