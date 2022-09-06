use std::fmt::{Debug, Formatter};

use beserial::{Deserialize, Serialize};
use nimiq_block::{Block, MacroBlock};
use nimiq_blockchain::HistoryTreeChunk;
use nimiq_hash::Blake2bHash;
use nimiq_network_interface::request::{RequestCommon, RequestMarker};

pub(crate) mod handlers;

/*
The consensus module uses the following messages:
200 RequestResponseMessage<RequestBlockHashes>
201 RequestResponseMessage<BlockHashes>
202 RequestResponseMessage<RequestEpoch>
203 RequestResponseMessage<Epoch>
*/

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

/// This message contains a macro block and the number of extended transactions (transitions)
/// within this epoch.
#[derive(Clone, Serialize, Deserialize)]
pub struct BatchSetInfo {
    pub block: Option<MacroBlock>,
    pub history_len: u32,
}

impl Debug for BatchSetInfo {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let mut debug_struct = f.debug_struct("BatchSetInfo");
        if let Some(block) = &self.block {
            debug_struct
                .field("epoch_number", &block.epoch_number())
                .field("block_number", &block.block_number())
                .field("is_election_block", &block.is_election_block());
        }
        debug_struct.field("history_len", &self.history_len);
        debug_struct.finish()
    }
}

/// This message contains a chunk of the history.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RequestHistoryChunk {
    pub epoch_number: u32,
    pub block_number: u32,
    pub chunk_index: u64,
}

impl RequestCommon for RequestHistoryChunk {
    type Kind = RequestMarker;
    const TYPE_ID: u16 = 204;
    type Response = HistoryChunk;
    const MAX_REQUESTS: u32 = MAX_REQUEST_RESPONSE_HISTORY_CHUNK;
}

/// This message contains a chunk of the history.
#[derive(Debug, Serialize, Deserialize)]
pub struct HistoryChunk {
    pub chunk: Option<HistoryTreeChunk>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RequestBlock {
    pub hash: Blake2bHash,
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
