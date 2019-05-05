use std::collections::HashSet;
use std::iter::FromIterator;

use tokio::prelude::*;
use futures::Future;

use hash::Blake2bHash;
use network_messages::{
    Message,
    GetBlockProofMessage,
    BlockProofMessage,
    GetTransactionReceiptsMessage,
    GetTransactionsProofMessage,
    TransactionReceiptsMessage,
    TransactionsProofMessage,
    GetAccountsProofMessage,
    AccountsProofMessage,
    GetAccountsTreeChunkMessage,
    AccountsTreeChunkMessage,
    AccountsTreeChunkData,
};
use network::connection::close_type::CloseType;

use crate::consensus_agent::ConsensusAgent;

impl ConsensusAgent {
    pub(super) fn on_get_chain_proof(&self) {
        trace!("[GET-CHAIN-PROOF] from {}", self.peer.peer_address());
        if !self.state.write().chain_proof_limit.note_single() {
            warn!("Rejecting GetChainProof message - rate-limit exceeded");
            self.peer.channel.close(CloseType::RateLimitExceeded);
            return;
        }

        let chain_proof = self.blockchain.get_chain_proof();
        self.peer.channel.send_or_close(Message::ChainProof(Box::new(chain_proof)));
    }

    pub(super) fn on_get_block_proof(&self, msg: GetBlockProofMessage) {
        trace!("[GET-BLOCK-PROOF] from {}", self.peer.peer_address());
        if !self.state.write().block_proof_limit.note_single() {
            warn!("Rejecting GetBlockProof message - rate-limit exceeded");
            self.peer.channel.send_or_close(BlockProofMessage::empty());
            return;
        }

        let block_proof = self.blockchain.get_block_proof(&msg.block_hash_to_prove, &msg.known_block_hash);
        self.peer.channel.send_or_close(BlockProofMessage::new(block_proof));
    }

    pub(super) fn on_get_transaction_receipts(&self, msg: GetTransactionReceiptsMessage) {
        trace!("[GET-TRANSACTION-RECEIPTS] from {}", self.peer.peer_address());
        if !self.state.write().transaction_receipts_limit.note_single() {
            warn!("Rejecting GetTransactionReceipts message - rate-limit exceeded");
            self.peer.channel.send_or_close(TransactionReceiptsMessage::empty());
            return;
        }

        let limit: usize = TransactionReceiptsMessage::RECEIPTS_MAX_COUNT / 2;
        let receipts = self.blockchain.get_transaction_receipts_by_address(&msg.address, limit, limit);
        self.peer.channel.send_or_close(TransactionReceiptsMessage::new(receipts));
    }

    pub(super) fn on_get_transactions_proof(&self, msg: GetTransactionsProofMessage) {
        trace!("[GET-TRANSACTIONS-PROOF] from {}", self.peer.peer_address());
        if !self.state.write().transactions_proof_limit.note_single() {
            warn!("Rejecting GetTransactionsProofMessage message - rate-limit exceeded");
            self.peer.channel.send_or_close(TransactionsProofMessage::new(msg.block_hash, None));
            return;
        }

        let addresses = HashSet::from_iter(msg.addresses);
        let proof = self.blockchain.get_transactions_proof(&msg.block_hash, &addresses);
        self.peer.channel.send_or_close(TransactionsProofMessage::new(msg.block_hash, proof));
    }

    pub(super) fn on_get_accounts_proof(&self, msg: GetAccountsProofMessage) {
        trace!("[GET-ACCOUNTS-PROOF] from {}", self.peer.peer_address());
        if !self.state.write().accounts_proof_limit.note_single() {
            warn!("Rejecting GetAccountsProof message - rate-limit exceeded");
            self.peer.channel.send_or_close(AccountsProofMessage::new(msg.block_hash, None));
            return;
        }

        // TODO: This is a deviation from the JavaScript client. If the given hash is the 0 hash, assume the current head.
        let mut hash = msg.block_hash;
        if hash == Blake2bHash::default() {
            hash = self.blockchain.head_hash();
        }

        let proof = self.blockchain.get_accounts_proof(&hash, &msg.addresses);
        self.peer.channel.send_or_close(AccountsProofMessage::new(hash, proof));
    }

    pub(super) fn on_get_accounts_tree_chunk(&self, msg: GetAccountsTreeChunkMessage) {
        trace!("[GET-ACCOUNTS-TREE-CHUNK] from {}", self.peer.peer_address());
        let get_chunk_future = self.accounts_chunk_cache.get_chunk(&msg.block_hash, &msg.start_prefix);
        let peer = self.peer.clone();
        let future = get_chunk_future.then(move |chunk_res| {
            let chunk_opt = chunk_res.unwrap_or(None).map(AccountsTreeChunkData::Serialized);
            peer.channel.send_or_close(Message::AccountsTreeChunk(Box::new(AccountsTreeChunkMessage { block_hash: msg.block_hash, chunk: chunk_opt })));
            future::ok::<(), ()>(())
        });
        tokio::spawn(future);
    }
}
