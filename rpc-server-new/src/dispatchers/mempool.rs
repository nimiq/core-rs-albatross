use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::RwLock;

use beserial::{Deserialize, Serialize};
use nimiq_hash::{Blake2bHash, Hash};
use nimiq_keys::Address;
use nimiq_mempool::{Mempool, ReturnCode};
use nimiq_primitives::account::AccountType;
use nimiq_primitives::coin::Coin;
use nimiq_transaction::{Transaction, TransactionFlags};

use crate::{wallets::UnlockedWallets, Error};

#[async_trait]
pub trait MempoolInterface {
    async fn send_raw_transaction(&self, raw_tx: String) -> Result<String, Error>;

    async fn create_raw_transaction(&self, tx_params: TransactionParameters) -> Result<String, Error>;

    async fn send_transaction(&self, tx_params: TransactionParameters) -> Result<String, Error>;

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

    fn push_transaction(&self, tx: Transaction) -> Result<Blake2bHash, Error> {
        let txid = tx.hash::<Blake2bHash>();
        match self.mempool.push_transaction(tx) {
            ReturnCode::Accepted | ReturnCode::Known => Ok(txid),
            code => Err(Error::TransactionRejected(code)),
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

#[nimiq_jsonrpc_derive::service(rename_all = "mixedCase")]
#[async_trait]
impl MempoolInterface for MempoolDispatcher {
    async fn send_raw_transaction(&self, raw_tx: String) -> Result<String, Error> {
        let tx = Deserialize::deserialize_from_vec(&hex::decode(&raw_tx)?)?;
        Ok(self.push_transaction(tx)?.to_hex())
    }

    async fn create_raw_transaction(&self, tx_params: TransactionParameters) -> Result<String, Error> {
        let mut tx = tx_params.into_transaction(&self.mempool)?;

        if tx.sender_type == AccountType::Basic {
            self.sign_transaction(&mut tx)?;
        }

        Ok(hex::encode(&tx.serialize_to_vec()))
    }

    async fn send_transaction(&self, tx_params: TransactionParameters) -> Result<String, Error> {
        let tx = tx_params.into_transaction(&self.mempool)?;
        Ok(self.push_transaction(tx)?.to_hex())
    }

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

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransactionParameters {
    from: Address,

    from_type: AccountType,

    to: Option<Address>,

    to_type: AccountType,

    value: Coin,

    fee: Coin,

    flags: TransactionFlags,

    #[serde(with = "crate::serde_helpers::hex")]
    data: Vec<u8>,

    validity_start_height: Option<u32>,
}

impl TransactionParameters {
    pub fn into_transaction(self, mempool: &Mempool) -> Result<Transaction, Error> {
        let validity_start_height = self.validity_start_height.unwrap_or_else(|| mempool.current_height());
        let network_id = mempool.network_id();

        match self.to {
            None if self.to_type != AccountType::Basic && self.flags.contains(TransactionFlags::CONTRACT_CREATION) => Ok(Transaction::new_contract_creation(
                self.data,
                self.from,
                self.from_type,
                self.to_type,
                self.value,
                self.fee,
                validity_start_height,
                network_id,
            )),
            Some(to) if !self.flags.contains(TransactionFlags::CONTRACT_CREATION) => Ok(Transaction::new_extended(
                self.from,
                self.from_type,
                to,
                self.to_type,
                self.value,
                self.fee,
                self.data,
                validity_start_height,
                network_id,
            )),
            _ => Err(Error::InvalidTransactionParameters),
        }
    }
}
