use std::sync::Arc;

use futures::{Stream, StreamExt};
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::broadcast::{
    channel as broadcast, Receiver as BroadcastReceiver, Sender as BroadcastSender,
};

use block_albatross::Block;
use blockchain_albatross::Blockchain;
use database::Environment;
use mempool::{Mempool, ReturnCode};
use network_interface::network::Network;
use transaction::Transaction;

use crate::consensus_agent::ConsensusAgent;
use crate::sync::block_queue::{BlockQueue, BlockQueueConfig, BlockTopic};
use crate::sync::request_component::BlockRequestComponent;
use futures::stream::BoxStream;
use futures::task::{Context, Poll};
use nimiq_network_interface::network::Topic;
use std::pin::Pin;

mod request_response;

#[derive(Clone, Debug, Default)]
pub struct TransactionTopic;

impl Topic for TransactionTopic {
    type Item = Transaction;

    fn topic(&self) -> String {
        "transactions".to_owned()
    }
}

pub struct ConsensusProxy<N: Network> {
    pub blockchain: Arc<Blockchain>,
    pub network: Arc<N>,
    pub mempool: Arc<Mempool>,
    established_flag: Arc<AtomicBool>,
}

impl<N: Network> Clone for ConsensusProxy<N> {
    fn clone(&self) -> Self {
        Self {
            blockchain: Arc::clone(&self.blockchain),
            network: Arc::clone(&self.network),
            mempool: Arc::clone(&self.mempool),
            established_flag: Arc::clone(&self.established_flag),
        }
    }
}

impl<N: Network> ConsensusProxy<N> {
    pub async fn send_transaction(&self, tx: Transaction) -> Result<ReturnCode, N::Error> {
        match self.mempool.push_transaction(tx.clone()) {
            ReturnCode::Accepted => {}
            e => return Ok(e),
        }

        self.network
            .publish(&TransactionTopic::default(), tx)
            .await
            .map(|_| ReturnCode::Accepted)
    }

    pub fn is_established(&self) -> bool {
        self.established_flag.load(Ordering::Acquire)
    }
}

pub enum ConsensusEvent<N: Network> {
    PeerJoined(Arc<ConsensusAgent<N::PeerType>>),
    //PeerLeft(Arc<P>),
    Established,
    Lost,
}

impl<N: Network> Clone for ConsensusEvent<N> {
    fn clone(&self) -> Self {
        match self {
            ConsensusEvent::PeerJoined(peer) => ConsensusEvent::PeerJoined(Arc::clone(peer)),
            ConsensusEvent::Established => ConsensusEvent::Established,
            ConsensusEvent::Lost => ConsensusEvent::Lost,
        }
    }
}

pub struct Consensus<N: Network> {
    pub blockchain: Arc<Blockchain>,
    pub mempool: Arc<Mempool>,
    pub network: Arc<N>,
    pub env: Environment,

    block_queue: BlockQueue<BlockRequestComponent<N::PeerType>>,
    tx_stream: BoxStream<'static, Transaction>,

    events: BroadcastSender<ConsensusEvent<N>>,
    established_flag: Arc<AtomicBool>,
}

impl<N: Network> Consensus<N> {
    /// Minimum number of peers for consensus to be established.
    const MIN_PEERS_ESTABLISHED: usize = 3;
    /// Minimum number of block announcements extending the chain for consensus to be established.
    const MIN_BLOCKS_ESTABLISHED: usize = 5;

    pub async fn from_network(
        env: Environment,
        blockchain: Arc<Blockchain>,
        mempool: Arc<Mempool>,
        network: Arc<N>,
        sync_protocol: BoxStream<'static, Arc<ConsensusAgent<N::PeerType>>>,
    ) -> Self {
        let block_stream = network
            .subscribe::<BlockTopic>(&BlockTopic::default())
            .await
            .unwrap()
            .map(|(block, _peer_id)| block)
            .boxed();

        let tx_stream = network
            .subscribe::<TransactionTopic>(&TransactionTopic::default())
            .await
            .unwrap()
            .map(|(tx, _peer_id)| tx)
            .boxed();

        Self::new(
            env,
            blockchain,
            mempool,
            network,
            block_stream,
            tx_stream,
            sync_protocol,
        )
    }

    pub fn new(
        env: Environment,
        blockchain: Arc<Blockchain>,
        mempool: Arc<Mempool>,
        network: Arc<N>,
        block_stream: BoxStream<'static, Block>,
        tx_stream: BoxStream<'static, Transaction>,
        sync_protocol: BoxStream<'static, Arc<ConsensusAgent<N::PeerType>>>,
    ) -> Self {
        let (tx, _rx) = broadcast(256);

        let request_component = BlockRequestComponent::new(sync_protocol);

        let block_queue = BlockQueue::new(
            BlockQueueConfig::default(),
            Arc::clone(&blockchain),
            request_component,
            block_stream,
        );

        Self::init_network_requests(&network, &blockchain);

        Consensus {
            blockchain,
            mempool,
            network,
            env,

            block_queue,
            tx_stream,
            events: tx,

            established_flag: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn subscribe_events(&self) -> BroadcastReceiver<ConsensusEvent<N>> {
        self.events.subscribe()
    }

    pub fn is_established(&self) -> bool {
        self.established_flag.load(Ordering::Acquire)
    }

    pub fn num_agents(&self) -> usize {
        self.block_queue.num_peers()
    }

    pub fn proxy(&self) -> ConsensusProxy<N> {
        ConsensusProxy {
            blockchain: Arc::clone(&self.blockchain),
            network: Arc::clone(&self.network),
            mempool: Arc::clone(&self.mempool),
            established_flag: Arc::clone(&self.established_flag),
        }
    }

    /// Calculates and sets established state, returns a ConsensusEvent if the state changed.
    fn set_established(&mut self) -> Option<ConsensusEvent<N>> {
        // We can only loose established state right now if we loose all our peers.
        if self.is_established() {
            if self.num_agents() == 0 {
                self.established_flag.swap(false, Ordering::Release);
                return Some(ConsensusEvent::Lost);
            }
        } else {
            if self.num_agents() >= Self::MIN_PEERS_ESTABLISHED
                && self.block_queue.accepted_block_announcements() >= Self::MIN_BLOCKS_ESTABLISHED
            {
                self.established_flag.swap(true, Ordering::Release);
                return Some(ConsensusEvent::Established);
            }
        }
        None
    }
}

impl<N: Network> Stream for Consensus<N> {
    type Item = ConsensusEvent<N>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        // 1. Poll and advance block queue
        while let Poll::Ready(Some(_)) = self.block_queue.poll_next_unpin(cx) {
            // TODO: Events?
            if let Some(event) = self.set_established() {
                return Poll::Ready(Some(event));
            }
        }

        // 2. Poll and push transactions once consensus is established.
        if self.is_established() {
            while let Poll::Ready(Some(tx)) = self.tx_stream.poll_next_unpin(cx) {
                // TODO: React on result.
                self.mempool.push_transaction(tx);
            }
        }

        Poll::Pending
    }
}
