use parking_lot::RwLock;
use std::sync::{Arc, Weak};
use std::time::Duration;

use crate::messages::{
    BlockHashes, Epoch, RequestBlockHashes, RequestBlockHashesFilter, RequestEpoch,
    RequestResponseMessage,
};
use block_albatross::Block;
use futures::{Future, StreamExt};
use hash::Blake2bHash;
use network_interface::peer::Peer;
use network_interface::request_response::{RequestError, RequestResponse};
use nimiq_subscription::Subscription;
use tokio;
use transaction::Transaction;
use utils::mutable_once::MutableOnce;
use utils::timers::Timers;

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

    /// Responses to `Blocks` requests.
    block_requests: RequestResponse<
        P,
        RequestResponseMessage<RequestBlockHashes>,
        RequestResponseMessage<BlockHashes>,
    >,

    epoch_requests:
        RequestResponse<P, RequestResponseMessage<RequestEpoch>, RequestResponseMessage<Epoch>>,
}

impl<P: Peer> ConsensusAgent<P> {
    pub fn new(peer: Arc<P>) -> Arc<Self> {
        // TODO: Timeout
        let block_requests = RequestResponse::new(Arc::clone(&peer), Duration::new(10, 0));
        let epoch_requests = RequestResponse::new(Arc::clone(&peer), Duration::new(10, 0));

        let agent = Arc::new(ConsensusAgent {
            peer,
            state: RwLock::new(ConsensusAgentState {
                local_subscription: Default::default(),
                remote_subscription: Default::default(),
            }),
            block_requests,
            epoch_requests,
        });
        agent
    }

    pub fn relay_block(&self, _block: &Block) -> bool {
        true
    }

    pub fn relay_transaction(&self, _transaction: &Transaction) -> bool {
        true
    }

    pub fn remove_transaction(&self, _transaction: &Transaction) {}

    pub fn request_block(
        &self,
        hash: Blake2bHash,
    ) -> impl Future<Output = Result<Block, RequestError>> + 'static {
        async { Err(RequestError::Timeout) }
    }

    pub async fn request_epoch(&self, hash: Blake2bHash) -> Result<Epoch, RequestError> {
        let result = self
            .epoch_requests
            .request(RequestResponseMessage::new(RequestEpoch { hash }))
            .await;

        result.map(|r| r.msg)
    }

    pub async fn request_blocks(
        &self,
        locators: Vec<Blake2bHash>,
        max_blocks: u16,
        filter: RequestBlockHashesFilter,
    ) -> Result<BlockHashes, RequestError> {
        let result = self
            .block_requests
            .request(RequestResponseMessage::new(RequestBlockHashes {
                locators,
                max_blocks,
                filter,
            }))
            .await;

        result.map(|r| r.msg)
    }
}
