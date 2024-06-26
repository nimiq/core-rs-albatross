use futures::stream::BoxStream;
use nimiq_block::Block;
use nimiq_network_interface::network::Network;
pub use proxy::BlockQueueProxy as BlockQueue;
use tokio::sync::oneshot::Sender as OneshotSender;

use crate::consensus::ResolveBlockError;

mod assembler;
pub mod block_request_component;
pub mod live_sync;
mod proxy;
mod queue;

pub type BlockStream<N> = BoxStream<
    'static,
    (
        Block,
        <N as Network>::PeerId,
        Option<<N as Network>::PubsubId>,
    ),
>;
pub type GossipSubBlockStream<N> = BoxStream<'static, (Block, <N as Network>::PubsubId)>;

pub type BlockAndId<N> = (Block, Option<<N as Network>::PubsubId>);

pub type ResolveBlockSender<N> = OneshotSender<Result<Block, ResolveBlockError<N>>>;

pub enum QueuedBlock<N: Network> {
    Head(BlockAndId<N>),
    Buffered(Vec<BlockAndId<N>>),
    Missing(Vec<Block>),
    TooFarAhead(Block, N::PeerId),
    TooFarBehind(Block, N::PeerId),
}
