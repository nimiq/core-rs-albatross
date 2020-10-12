use beserial::{Deserialize, Serialize};
use block_albatross::MacroBlock;
use blockchain_albatross::history_store::HistoryTreeChunk;
use failure::_core::fmt::{Error, Formatter};
use hash::Blake2bHash;
use network_interface::message::*;
use std::fmt::Debug;
use transaction::Transaction;

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

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BlockHashes {
    #[beserial(len_type(u16))]
    pub hashes: Vec<Blake2bHash>,
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
pub struct RequestEpoch {
    pub hash: Blake2bHash,
    pub request_identifier: u32,
}
request_response!(RequestEpoch);

impl Message for RequestEpoch {
    const TYPE_ID: u64 = 202;
}

/// This message contains a macro block and the number of extended transactions (transitions)
/// within this epoch.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Epoch {
    pub block: MacroBlock,
    pub history_len: u64,
    pub request_identifier: u32,
}
request_response!(Epoch);

impl Message for Epoch {
    const TYPE_ID: u64 = 203;
}

/// This message contains a chunk of the history.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RequestHistoryChunk {
    pub epoch_number: u32,
    pub chunk_index: u64,
    pub request_identifier: u32,
}
request_response!(RequestHistoryChunk);

impl Message for RequestHistoryChunk {
    const TYPE_ID: u64 = 204;
}

/// This message contains a chunk of the history.
#[derive(Serialize, Deserialize)]
pub struct HistoryChunk {
    chunk: Option<HistoryTreeChunk>,
    pub request_identifier: u32,
}
request_response!(HistoryChunk);

impl Message for HistoryChunk {
    const TYPE_ID: u64 = 205;
}

impl Debug for HistoryChunk {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result<(), Error> {
        unimplemented!()
    }
}
