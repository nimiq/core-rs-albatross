use std::sync::Arc;

use nimiq_hash::Blake2bHash;
use nimiq_network_interface::{
    network::Network,
    request::{Handle, RequestCommon, RequestMarker},
};
use nimiq_serde::{Deserialize, Serialize};
use nimiq_transaction::Transaction;
use parking_lot::RwLock;

use crate::mempool_state::MempoolState;

const MAX_REQUEST_RESPONSE_MEMPOOL_STATE: u32 = 1000;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum MempoolTransactionType {
    Control,
    Regular,
}

/// Request the current transaction hashes in the mempool.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RequestMempoolHashes {
    pub transaction_type: MempoolTransactionType,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ResponseMempoolHashes {
    pub hashes: Vec<Blake2bHash>,
}

impl RequestCommon for RequestMempoolHashes {
    type Kind = RequestMarker;
    const TYPE_ID: u16 = 219;
    type Response = ResponseMempoolHashes;
    const MAX_REQUESTS: u32 = MAX_REQUEST_RESPONSE_MEMPOOL_STATE;
}

/// Request transactions in the mempool based on the provided hashes.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RequestMempoolTransactions {
    pub hashes: Vec<Blake2bHash>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ResponseMempoolTransactions {
    pub transactions: Vec<Transaction>,
}

impl RequestCommon for RequestMempoolTransactions {
    type Kind = RequestMarker;
    const TYPE_ID: u16 = 220;
    type Response = ResponseMempoolTransactions;
    const MAX_REQUESTS: u32 = MAX_REQUEST_RESPONSE_MEMPOOL_STATE;
}

impl<N: Network> Handle<N, Arc<RwLock<MempoolState>>> for RequestMempoolHashes {
    fn handle(&self, _: N::PeerId, context: &Arc<RwLock<MempoolState>>) -> ResponseMempoolHashes {
        let hashes: Vec<Blake2bHash> = match self.transaction_type {
            MempoolTransactionType::Regular => context
                .read()
                .regular_transactions
                .best_transactions
                .iter()
                .map(|txn| txn.0.clone())
                .collect(),
            MempoolTransactionType::Control => context
                .read()
                .control_transactions
                .best_transactions
                .iter()
                .map(|txn| txn.0.clone())
                .collect(),
        };

        ResponseMempoolHashes { hashes }
    }
}

impl<N: Network> Handle<N, Arc<RwLock<MempoolState>>> for RequestMempoolTransactions {
    fn handle(
        &self,
        _: N::PeerId,
        context: &Arc<RwLock<MempoolState>>,
    ) -> ResponseMempoolTransactions {
        let mut transactions = Vec::with_capacity(self.hashes.len());
        let state = context.read();
        self.hashes.iter().for_each(|hash| {
            if let Some(txn) = state.get(hash) {
                transactions.push(txn.to_owned());
            }
        });

        ResponseMempoolTransactions { transactions }
    }
}
