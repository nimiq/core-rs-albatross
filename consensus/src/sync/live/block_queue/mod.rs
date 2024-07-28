use futures::stream::BoxStream;
use nimiq_block::Block;
use nimiq_network_interface::network::{MsgAcceptance, Network, PubsubId};
pub use proxy::BlockQueueProxy as BlockQueue;
use tokio::sync::oneshot::Sender as OneshotSender;

use crate::{
    consensus::ResolveBlockError,
    messages::{BlockBodyTopic, BlockHeaderTopic},
};

mod assembler;
pub mod block_request_component;
pub mod live_sync;
mod proxy;
mod queue;

pub type BlockStream<N> = BoxStream<'static, BlockAndSource<N>>;
pub type GossipSubBlockStream<N> = BoxStream<'static, (Block, <N as Network>::PubsubId)>;

pub type BlockAndSource<N> = (Block, BlockSource<N>);

pub type ResolveBlockSender<N> = OneshotSender<Result<Block, ResolveBlockError<N>>>;

pub enum QueuedBlock<N: Network> {
    Head(BlockAndSource<N>),
    Buffered(Vec<BlockAndSource<N>>),
    Missing(Vec<BlockAndSource<N>>),
    TooFarAhead(N::PeerId),
    TooFarBehind(N::PeerId),
}

#[derive(Debug)]
pub enum BlockSource<N: Network> {
    Announced {
        header_id: N::PubsubId,
        body_id: Option<N::PubsubId>,
    },
    Requested {
        id: N::PeerId,
    },
}

impl<N: Network> BlockSource<N> {
    pub fn announced(header_id: N::PubsubId, body_id: Option<N::PubsubId>) -> Self {
        Self::Announced { header_id, body_id }
    }

    pub fn requested(id: N::PeerId) -> Self {
        Self::Requested { id }
    }

    pub fn peer_id(&self) -> N::PeerId {
        match self {
            BlockSource::Announced { header_id, .. } => header_id.propagation_source(),
            BlockSource::Requested { id } => id.clone(),
        }
    }

    pub fn is_announced(&self) -> bool {
        matches!(self, BlockSource::Announced { .. })
    }

    pub fn is_requested(&self) -> bool {
        matches!(self, BlockSource::Requested { .. })
    }

    pub fn accept_block(&self, network: &N) {
        self.validate_block(network, MsgAcceptance::Accept);
    }

    pub fn reject_block(&self, network: &N) {
        self.validate_block(network, MsgAcceptance::Reject);
    }

    pub fn ignore_block(&self, network: &N) {
        self.validate_block(network, MsgAcceptance::Ignore);
    }

    pub fn validate_block(&self, network: &N, acceptance: MsgAcceptance) {
        match self {
            BlockSource::Announced { header_id, body_id } => {
                network.validate_message::<BlockHeaderTopic>(header_id.clone(), acceptance);
                if let Some(body_id) = body_id {
                    network.validate_message::<BlockBodyTopic>(body_id.clone(), acceptance);
                }
            }
            BlockSource::Requested { .. } => {}
        }
    }
}

impl<N: Network> Clone for BlockSource<N> {
    fn clone(&self) -> Self {
        match self {
            BlockSource::Requested { id } => BlockSource::Requested { id: *id },
            BlockSource::Announced { header_id, body_id } => BlockSource::Announced {
                header_id: header_id.clone(),
                body_id: body_id.clone(),
            },
        }
    }
}
