use std::fmt::{Debug, Formatter};

use beserial::{Deserialize, Serialize};
use nimiq_block::{Block, MacroBlock};
#[cfg(feature = "full")]
use nimiq_blockchain::HistoryTreeChunk;
use nimiq_hash::Blake2bHash;
use nimiq_keys::Address;
use nimiq_network_interface::request::{RequestCommon, RequestMarker};
use nimiq_primitives::trie::trie_proof::TrieProof;
use nimiq_transaction::history_proof::HistoryTreeProof;

mod handlers;

/*
The consensus module uses the following messages:
200 RequestResponseMessage<RequestBlockHashes>
201 RequestResponseMessage<BlockHashes>
202 RequestResponseMessage<RequestEpoch>
203 RequestResponseMessage<Epoch>
*/

// The max number of Accounts proof requests per peer.
pub const MAX_REQUEST_ACCOUNTS_PROOF: u32 = 1000;
/// The max number of MacroChain requests per peer.
pub const MAX_REQUEST_RESPONSE_MACRO_CHAIN: u32 = 1000;
/// The max number of BatchSet requests per peer.
pub const MAX_REQUEST_RESPONSE_BATCH_SET: u32 = 1000;
/// The max number of HistoryChunk requests per peer.
pub const MAX_REQUEST_RESPONSE_HISTORY_CHUNK: u32 = 1000;
/// The max number of RequestBlock requests per peer.
pub const MAX_REQUEST_RESPONSE_BLOCK: u32 = 1000;
/// The max number of MissingBlocks requests per peer.
pub const MAX_REQUEST_RESPONSE_MISSING_BLOCKS: u32 = 1000;
/// The max number of RequestHead requests per peer.
pub const MAX_REQUEST_RESPONSE_HEAD: u32 = 1000;
/// The max number of Transactions proof requests per peer.
pub const MAX_REQUEST_TRANSACTIONS_PROOF: u32 = 1000;
/// The max number of Transactions proof requests per peer.
pub const MAX_REQUEST_TRANSACTIONS_BY_ADDRESS: u32 = 1000;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Checkpoint {
    pub block_number: u32,
    pub hash: Blake2bHash,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct MacroChain {
    #[beserial(len_type(u16))]
    pub epochs: Option<Vec<Blake2bHash>>,
    pub checkpoint: Option<Checkpoint>,
}

impl Debug for MacroChain {
    fn fmt(&self, f: &mut Formatter) -> std::fmt::Result {
        let mut dbg = f.debug_struct("MacroChain");
        if let Some(epochs) = &self.epochs {
            let len = epochs.len();
            dbg.field("num_epochs", &len);
            if !epochs.is_empty() {
                let first = epochs.first().unwrap();
                let last = epochs.last().unwrap();
                dbg.field("first_epoch", &first);
                dbg.field("last_epoch", &last);
            }
        }
        dbg.field("checkpoint", &self.checkpoint);
        dbg.finish()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RequestMacroChain {
    #[beserial(len_type(u16, limit = 128))]
    pub locators: Vec<Blake2bHash>,
    pub max_epochs: u16,
}

impl RequestCommon for RequestMacroChain {
    type Kind = RequestMarker;
    const TYPE_ID: u16 = 200;
    type Response = MacroChain;
    const MAX_REQUESTS: u32 = MAX_REQUEST_RESPONSE_MACRO_CHAIN;
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RequestBatchSet {
    pub hash: Blake2bHash,
}

impl RequestCommon for RequestBatchSet {
    type Kind = RequestMarker;
    const TYPE_ID: u16 = 202;
    type Response = BatchSetInfo;
    const MAX_REQUESTS: u32 = MAX_REQUEST_RESPONSE_BATCH_SET;
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BatchSet {
    pub macro_block: Option<MacroBlock>,
    pub history_len: u32,
}

/// This message contains a macro block and the number of extended transactions (transitions)
/// within this epoch.
#[derive(Clone, Serialize, Deserialize)]
pub struct BatchSetInfo {
    pub election_macro_block: Option<MacroBlock>,
    #[beserial(len_type(u16, limit = 128))]
    pub batch_sets: Vec<BatchSet>,
    pub total_history_len: u64,
}

impl Debug for BatchSetInfo {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let mut debug_struct = f.debug_struct("BatchSetInfo");
        if let Some(block) = &self.election_macro_block {
            debug_struct
                .field("election_epoch_number", &block.epoch_number())
                .field("election_block_number", &block.block_number());
        }
        debug_struct.field("total_history_len", &self.total_history_len);
        debug_struct.field("batch_sets_len", &self.batch_sets.len());
        debug_struct.finish()
    }
}

#[cfg(feature = "full")]
/// This message contains a chunk of the history.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RequestHistoryChunk {
    pub epoch_number: u32,
    pub block_number: u32,
    pub chunk_index: u64,
}

#[cfg(feature = "full")]
impl RequestCommon for RequestHistoryChunk {
    type Kind = RequestMarker;
    const TYPE_ID: u16 = 204;
    type Response = HistoryChunk;
    const MAX_REQUESTS: u32 = MAX_REQUEST_RESPONSE_HISTORY_CHUNK;
}

#[cfg(feature = "full")]
/// This message contains a chunk of the history.
#[derive(Debug, Serialize, Deserialize)]
pub struct HistoryChunk {
    pub chunk: Option<HistoryTreeChunk>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RequestBlock {
    pub hash: Blake2bHash,
    pub include_micro_bodies: bool,
}

impl RequestCommon for RequestBlock {
    type Kind = RequestMarker;
    const TYPE_ID: u16 = 207;
    type Response = Option<Block>;
    const MAX_REQUESTS: u32 = MAX_REQUEST_RESPONSE_BLOCK;
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ResponseBlocks {
    // TODO: Set to sensible limit (2 * BATCH_SIZE for example).
    #[beserial(len_type(u16, limit = 256))]
    pub blocks: Option<Vec<Block>>,
}

impl Debug for ResponseBlocks {
    fn fmt(&self, f: &mut Formatter) -> std::fmt::Result {
        let mut dbg = f.debug_struct("ResponseBlocks");
        if let Some(blocks) = &self.blocks {
            let len = blocks.len();
            dbg.field("length", &len);
            if !blocks.is_empty() {
                let first = blocks.first().unwrap();
                let last = blocks.last().unwrap();
                dbg.field("first_block", &first.block_number());
                dbg.field("last_block", &last.block_number());
            }
        }
        dbg.finish()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RequestMissingBlocks {
    pub target_hash: Blake2bHash,
    pub include_micro_bodies: bool,
    #[beserial(len_type(u16, limit = 128))]
    pub locators: Vec<Blake2bHash>,
}

impl RequestCommon for RequestMissingBlocks {
    type Kind = RequestMarker;
    const TYPE_ID: u16 = 209;
    type Response = ResponseBlocks;
    const MAX_REQUESTS: u32 = MAX_REQUEST_RESPONSE_MISSING_BLOCKS;
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RequestHead {}

impl RequestCommon for RequestHead {
    type Kind = RequestMarker;
    const TYPE_ID: u16 = 210;
    type Response = Blake2bHash;
    const MAX_REQUESTS: u32 = MAX_REQUEST_RESPONSE_HEAD;
}

#[derive(Serialize, Deserialize)]
pub struct ResponseTransactionsProof {
    pub proof: Option<HistoryTreeProof>,
    pub block: Option<Block>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RequestTransactionsProof {
    #[beserial(len_type(u16, limit = 128))]
    pub hashes: Vec<Blake2bHash>,
    pub block_number: u32,
}

impl RequestCommon for RequestTransactionsProof {
    type Kind = RequestMarker;
    const TYPE_ID: u16 = 213;
    type Response = ResponseTransactionsProof;
    const MAX_REQUESTS: u32 = MAX_REQUEST_TRANSACTIONS_PROOF;
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RequestTransactionReceiptsByAddress {
    pub address: Address,
    pub max: Option<u16>,
}

impl RequestCommon for RequestTransactionReceiptsByAddress {
    type Kind = RequestMarker;
    const TYPE_ID: u16 = 214;
    type Response = ResponseTransactionReceiptsByAddress;
    const MAX_REQUESTS: u32 = MAX_REQUEST_TRANSACTIONS_BY_ADDRESS;
}

#[derive(Serialize, Deserialize)]
pub struct ResponseTransactionReceiptsByAddress {
    /// Tuples of `(transaction_hash, block_number)`
    #[beserial(len_type(u16, limit = 128))]
    pub receipts: Vec<(Blake2bHash, u32)>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RequestAccountsProof {
    #[beserial(len_type(u16, limit = 128))]
    /// Addresses for which the accounts trie proof is requested for
    pub addresses: Vec<Address>,
}

impl RequestCommon for RequestAccountsProof {
    type Kind = RequestMarker;
    const TYPE_ID: u16 = 215;
    type Response = ResponseAccountsProof;
    const MAX_REQUESTS: u32 = MAX_REQUEST_ACCOUNTS_PROOF;
}

#[derive(Serialize, Deserialize)]
pub struct ResponseAccountsProof {
    // The accounts proof
    pub proof: Option<TrieProof>,
    // The hash of the block that was used to create the proof
    pub block_hash: Option<Blake2bHash>,
}
