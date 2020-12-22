use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::RwLock;

use nimiq_hash::Blake2bHash;
use nimiq_mempool::Mempool;
use nimiq_rpc_interface::{
    mempool::MempoolInterface,
};

use crate::{
    wallets::UnlockedWallets,
    error::Error,
};


#[allow(dead_code)]
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

#[nimiq_jsonrpc_derive::service(rename_all="camelCase")]
#[async_trait]
impl MempoolInterface for MempoolDispatcher {
    type Error = Error;

    async fn get_transaction(&mut self, _txid: Blake2bHash) -> Result<Option<()>, Error> {
        /*Ok(self.mempool
        .get_transaction(&txid)
        // TODO: We can return a `Arc<Transaction>`, if we implement `serde::Serialize` for it.
        .map(|tx| Transaction::clone(&tx)))*/
        Err(Error::NotImplemented)
    }

    async fn mempool_content(&mut self, _include_transactions: bool) -> Result<Vec<()>, Error> {
        Err(Error::NotImplemented)
    }

    async fn mempool(&mut self) -> Result<(), Error> {
        Err(Error::NotImplemented)
    }

    async fn get_mempool_transaction(&mut self) -> Result<(), Error> {
        Err(Error::NotImplemented)
    }
}
