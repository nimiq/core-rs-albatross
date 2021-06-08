use beserial::{Deserialize, Serialize};
use block::{Block, MacroBlock};
use blockchain::HistoryTreeChunk;
use hash::Blake2bHash;
use network_interface::message::*;
use std::fmt::{Debug, Error, Formatter};

use crate::request_response;

pub(crate) mod handlers;
mod request_response;

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

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BlockHashes {
    #[beserial(len_type(u16))]
    pub hashes: Option<Vec<(BlockHashType, Blake2bHash)>>,
    pub request_identifier: u32,
}
request_response!(BlockHashes);

impl Message for BlockHashes {
    const TYPE_ID: u64 = 201;
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
    pub request_identifier: u32,
}
request_response!(RequestBlockHashes);

impl Message for RequestBlockHashes {
    const TYPE_ID: u64 = 200;
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RequestBatchSet {
    pub hash: Blake2bHash,
    pub request_identifier: u32,
}
request_response!(RequestBatchSet);

impl Message for RequestBatchSet {
    const TYPE_ID: u64 = 202;
}

/// This message contains a macro block and the number of extended transactions (transitions)
/// within this epoch.
#[derive(Clone, Serialize, Deserialize)]
pub struct BatchSetInfo {
    pub block: Option<MacroBlock>,
    pub history_len: u32,
    pub request_identifier: u32,
}
request_response!(BatchSetInfo);

impl Message for BatchSetInfo {
    const TYPE_ID: u64 = 203;
}

impl Debug for BatchSetInfo {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result<(), Error> {
        let mut debug_struct = f.debug_struct("BatchSetInfo");
        if let Some(block) = &self.block {
            debug_struct
                .field("epoch_number", &block.epoch_number())
                .field("block_number", &block.block_number())
                .field("is_election_block", &block.is_election_block());
        }
        debug_struct
            .field("history_len", &self.history_len)
            .field("request_identifier", &self.request_identifier);
        debug_struct.finish()
    }
}

/// This message contains a chunk of the history.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RequestHistoryChunk {
    pub epoch_number: u32,
    pub block_number: u32,
    pub chunk_index: u64,
    pub request_identifier: u32,
}
request_response!(RequestHistoryChunk);

impl Message for RequestHistoryChunk {
    const TYPE_ID: u64 = 204;
}

/// This message contains a chunk of the history.
#[derive(Debug, Serialize, Deserialize)]
pub struct HistoryChunk {
    pub chunk: Option<HistoryTreeChunk>,
    pub request_identifier: u32,
}
request_response!(HistoryChunk);

impl Message for HistoryChunk {
    const TYPE_ID: u64 = 205;
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ResponseBlock {
    pub block: Option<Block>,
    pub request_identifier: u32,
}
request_response!(ResponseBlock);

impl Message for ResponseBlock {
    const TYPE_ID: u64 = 206;
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RequestBlock {
    pub hash: Blake2bHash,
    pub request_identifier: u32,
}
request_response!(RequestBlock);

impl Message for RequestBlock {
    const TYPE_ID: u64 = 207;
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ResponseBlocks {
    // TODO: Set to sensible limit (2 * BATCH_SIZE for example).
    #[beserial(len_type(u16, limit = 256))]
    pub blocks: Option<Vec<Block>>,
    pub request_identifier: u32,
}
request_response!(ResponseBlocks);

impl Message for ResponseBlocks {
    const TYPE_ID: u64 = 208;
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RequestMissingBlocks {
    pub target_hash: Blake2bHash,
    #[beserial(len_type(u16, limit = 128))]
    pub locators: Vec<Blake2bHash>,
    pub request_identifier: u32,
}
request_response!(RequestMissingBlocks);

impl Message for RequestMissingBlocks {
    const TYPE_ID: u64 = 209;
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RequestHead {
    pub request_identifier: u32,
}
request_response!(RequestHead);

impl Message for RequestHead {
    const TYPE_ID: u64 = 210;
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HeadResponse {
    pub hash: Blake2bHash,
    pub request_identifier: u32,
}
request_response!(HeadResponse);

impl Message for HeadResponse {
    const TYPE_ID: u64 = 211;
}
