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
#[repr(u8)]
pub enum Objects<T: Serialize + Deserialize> {
    #[beserial(discriminant = 0)]
    Hashes(#[beserial(len_type(u16))] Vec<Blake2bHash>),
    #[beserial(discriminant = 1)]
    Objects(#[beserial(len_type(u16))] Vec<T>),
}

impl<T: Serialize + Deserialize> Objects<T> {
    pub const MAX_HASHES: usize = 1000;
    pub const MAX_OBJECTS: usize = 1000;

    pub fn with_objects(objects: Vec<T>) -> Self {
        Objects::Objects(objects)
    }

    pub fn with_hashes(hashes: Vec<Blake2bHash>) -> Self {
        Objects::Hashes(hashes)
    }

    pub fn contains_hashes(&self) -> bool {
        matches!(self, Objects::Hashes(_))
    }

    pub fn contains_objects(&self) -> bool {
        !self.contains_hashes()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[repr(u8)]
pub enum BlockHashType {
    Micro = 1,
    Checkpoint = 2,
    Election = 3,
}

impl<'a> From<&'a Block> for BlockHashType {
    fn from(block: &'a Block) -> Self {
        match block {
            Block::Micro(_) => BlockHashType::Micro,
            Block::Macro(macro_block) => {
                if macro_block.is_election_block() {
                    BlockHashType::Election
                } else {
                    BlockHashType::Checkpoint
                }
            }
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct BlockHashes {
    #[beserial(len_type(u16))]
    pub hashes: Option<Vec<(BlockHashType, Blake2bHash)>>,
}

impl Message for BlockHashes {
    const TYPE_ID: u64 = 201;
}

impl Debug for BlockHashes {
    fn fmt(&self, f: &mut Formatter) -> std::fmt::Result {
        let mut dbg = f.debug_struct("BlockHashes");
        if let Some(hashes) = &self.hashes {
            let len = hashes.len();
            dbg.field("length", &len);
            if !hashes.is_empty() {
                let first = hashes.first().unwrap();
                let last = hashes.last().unwrap();
                dbg.field("first_hash", &first);
                dbg.field("last_hash", &last);
            }
        }
        dbg.finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[repr(u8)]
pub enum RequestBlockHashesFilter {
    All = 1,
    ElectionOnly = 2,
    ElectionAndLatestCheckpoint = 3,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RequestBlockHashes {
    #[beserial(len_type(u16, limit = 128))]
    pub locators: Vec<Blake2bHash>,
    pub max_blocks: u16,
    pub filter: RequestBlockHashesFilter,
}

impl Message for RequestBlockHashes {
    const TYPE_ID: u64 = 200;
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RequestBatchSet {
    pub hash: Blake2bHash,
}

impl Message for RequestBatchSet {
    const TYPE_ID: u64 = 202;
}

/// This message contains a macro block and the number of extended transactions (transitions)
/// within this epoch.
#[derive(Clone, Serialize, Deserialize)]
pub struct BatchSetInfo {
    pub block: Option<MacroBlock>,
    pub history_len: u32,
}

impl Message for BatchSetInfo {
    const TYPE_ID: u64 = 203;
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
    const TYPE_ID: u64 = 204;
}

/// This message contains a chunk of the history.
#[derive(Debug, Serialize, Deserialize)]
pub struct HistoryChunk {
    pub chunk: Option<HistoryTreeChunk>,
}

impl Message for HistoryChunk {
    const TYPE_ID: u64 = 205;
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ResponseBlock {
    pub block: Option<Block>,
}

impl Message for ResponseBlock {
    const TYPE_ID: u64 = 206;
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
    const TYPE_ID: u64 = 207;
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ResponseBlocks {
    // TODO: Set to sensible limit (2 * BATCH_SIZE for example).
    #[beserial(len_type(u16, limit = 256))]
    pub blocks: Option<Vec<Block>>,
}

impl Message for ResponseBlocks {
    const TYPE_ID: u64 = 208;
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
    const TYPE_ID: u64 = 209;
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RequestHead {}

impl Message for RequestHead {
    const TYPE_ID: u64 = 210;
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HeadResponse {
    pub hash: Blake2bHash,
}

impl Message for HeadResponse {
    const TYPE_ID: u64 = 211;
}
