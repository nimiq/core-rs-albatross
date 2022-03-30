use std::fmt::{Debug, Formatter};

use beserial::{Deserialize, Serialize};
use nimiq_block::{Block, MacroBlock};
use nimiq_blockchain::HistoryTreeChunk;
use nimiq_hash::Blake2bHash;
use nimiq_network_interface::message::*;

pub(crate) mod handlers;

/*
The consensus module uses the following messages:
200 RequestResponseMessage<RequestBlockHashes>
201 RequestResponseMessage<BlockHashes>
202 RequestResponseMessage<RequestEpoch>
203 RequestResponseMessage<Epoch>
*/

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

impl Message for MacroChain {
    const TYPE_ID: MessageTypeId = MessageTypeId::MacroChain;
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

impl Message for RequestMacroChain {
    const TYPE_ID: MessageTypeId = MessageTypeId::RequestMacroChain;
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RequestBatchSet {
    pub hash: Blake2bHash,
}

impl Message for RequestBatchSet {
    const TYPE_ID: MessageTypeId = MessageTypeId::RequestBatchSet;
}

/// This message contains a macro block and the number of extended transactions (transitions)
/// within this epoch.
#[derive(Clone, Serialize, Deserialize)]
pub struct BatchSetInfo {
    pub block: Option<MacroBlock>,
    pub history_len: u32,
}

impl Message for BatchSetInfo {
    const TYPE_ID: MessageTypeId = MessageTypeId::BatchSetInfo;
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

impl Message for RequestHistoryChunk {
    const TYPE_ID: MessageTypeId = MessageTypeId::RequestHistoryChunk;
}

/// This message contains a chunk of the history.
#[derive(Debug, Serialize, Deserialize)]
pub struct HistoryChunk {
    pub chunk: Option<HistoryTreeChunk>,
}

impl Message for HistoryChunk {
    const TYPE_ID: MessageTypeId = MessageTypeId::HistoryChunk;
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ResponseBlock {
    pub block: Option<Block>,
}

impl Message for ResponseBlock {
    const TYPE_ID: MessageTypeId = MessageTypeId::ResponseBlock;
}

impl Debug for ResponseBlock {
    fn fmt(&self, f: &mut Formatter) -> std::fmt::Result {
        let mut dbg = f.debug_struct("ResponseBlock");
        if let Some(block) = &self.block {
            dbg.field("hash", &block.hash());
            dbg.field("header", &block.header());
        }
        dbg.finish()
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RequestBlock {
    pub hash: Blake2bHash,
}

impl Message for RequestBlock {
    const TYPE_ID: MessageTypeId = MessageTypeId::RequestBlock;
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ResponseBlocks {
    // TODO: Set to sensible limit (2 * BATCH_SIZE for example).
    #[beserial(len_type(u16, limit = 256))]
    pub blocks: Option<Vec<Block>>,
}

impl Message for ResponseBlocks {
    const TYPE_ID: MessageTypeId = MessageTypeId::ResponseBlocks;
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

impl Message for RequestMissingBlocks {
    const TYPE_ID: MessageTypeId = MessageTypeId::RequestMissingBlocks;
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RequestHead {}

impl Message for RequestHead {
    const TYPE_ID: MessageTypeId = MessageTypeId::RequestHead;
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HeadResponse {
    pub hash: Blake2bHash,
}

impl Message for HeadResponse {
    const TYPE_ID: MessageTypeId = MessageTypeId::HeadResponse;
}
