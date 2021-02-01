use std::fmt::{Debug, Formatter, Result as FmtResult};
use std::sync::Arc;
use std::time::Duration;

use parking_lot::RwLock;

use block_albatross::Block;
use hash::Blake2bHash;
use network_interface::peer::Peer;
use network_interface::request_response::{RequestError, RequestResponse};
use nimiq_subscription::Subscription;

use crate::messages::*;

pub struct ConsensusAgentState {
    local_subscription: Subscription,
    remote_subscription: Subscription,
}

#[derive(Ord, PartialOrd, PartialEq, Eq, Hash, Clone, Copy, Debug)]
enum ConsensusAgentTimer {
    Mempool,
    ResyncThrottle,
    RequestTimeout(u32),
}

pub struct ConsensusAgent<P: Peer> {
    pub peer: Arc<P>,

    pub(crate) state: RwLock<ConsensusAgentState>,

    block_hashes_requests: RequestResponse<P, RequestBlockHashes, BlockHashes>,
    epoch_requests: RequestResponse<P, RequestBatchSet, BatchSetInfo>,
    history_chunk_requests: RequestResponse<P, RequestHistoryChunk, HistoryChunk>,
    block_requests: RequestResponse<P, RequestBlock, ResponseBlock>,
    missing_block_requests: RequestResponse<P, RequestMissingBlocks, ResponseBlocks>,
    head_requests: RequestResponse<P, RequestHead, HeadResponse>,
}

impl<P: Peer> Debug for ConsensusAgent<P> {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        self.peer.id().fmt(f)
    }
}

impl<P: Peer> ConsensusAgent<P> {
    pub fn new(peer: Arc<P>) -> Self {
        // TODO: Timeout
        // NOTE: Set this very high for now. When starting a small test-net the first request
        // can easily timeout.
        let timeout = Duration::from_secs(60);
        let block_hashes_requests = RequestResponse::new(Arc::clone(&peer), timeout);
        let epoch_requests = RequestResponse::new(Arc::clone(&peer), timeout);
        let history_chunk_requests = RequestResponse::new(Arc::clone(&peer), timeout);
        let block_requests = RequestResponse::new(Arc::clone(&peer), timeout);
        let missing_block_requests = RequestResponse::new(Arc::clone(&peer), timeout);
        let head_requests = RequestResponse::new(Arc::clone(&peer), timeout);

        ConsensusAgent {
            peer,
            state: RwLock::new(ConsensusAgentState {
                local_subscription: Default::default(),
                remote_subscription: Default::default(),
            }),
            block_hashes_requests,
            epoch_requests,
            history_chunk_requests,
            block_requests,
            missing_block_requests,
            head_requests,
        }
    }

    pub async fn request_block(&self, hash: Blake2bHash) -> Result<Option<Block>, RequestError> {
        let result = self
            .block_requests
            .request(RequestBlock {
                hash,
                request_identifier: 0, // will automatically be set at a later point
            })
            .await;

        result.map(|response_block| response_block.block)
    }

    pub async fn request_epoch(&self, hash: Blake2bHash) -> Result<BatchSetInfo, RequestError> {
        let result = self
            .epoch_requests
            .request(RequestBatchSet {
                hash,
                request_identifier: 0, // will automatically be set at a later point
            })
            .await;

        // TODO verify that hash of returned epoch matches the one we requested

        result
    }

    pub async fn request_block_hashes(
        &self,
        locators: Vec<Blake2bHash>,
        max_blocks: u16,
        filter: RequestBlockHashesFilter,
    ) -> Result<BlockHashes, RequestError> {
        let result = self
            .block_hashes_requests
            .request(RequestBlockHashes {
                locators,
                max_blocks,
                filter,
                request_identifier: 0, // will automatically be set at a later point
            })
            .await;

        result
    }

    pub async fn request_history_chunk(
        &self,
        epoch_number: u32,
        chunk_index: usize,
    ) -> Result<HistoryChunk, RequestError> {
        let result = self
            .history_chunk_requests
            .request(RequestHistoryChunk {
                epoch_number,
                chunk_index: chunk_index as u64,
                request_identifier: 0, // will automatically be set at a later point
            })
            .await;

        // TODO filter empty chunks here?

        result
    }

    pub async fn request_missing_blocks(
        &self,
        target_block_hash: Blake2bHash,
        locators: Vec<Blake2bHash>,
    ) -> Result<Vec<Block>, RequestError> {
        let result = self
            .missing_block_requests
            .request(RequestMissingBlocks {
                locators,
                target_hash: target_block_hash,
                request_identifier: 0, // will automatically be set at a later point
            })
            .await;

        result.map(|response_blocks| response_blocks.blocks)
    }

    pub async fn request_head(&self) -> Result<Blake2bHash, RequestError> {
        let result = self
            .head_requests
            .request(RequestHead {
                request_identifier: 0, // will automatically be set at a later point
            })
            .await;

        result.map(|response_blocks| response_blocks.hash)
    }
}
