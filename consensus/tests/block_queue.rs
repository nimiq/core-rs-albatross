use std::{
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use futures::{
    channel::mpsc,
    sink::SinkExt,
    stream::{Stream, StreamExt},
};
use pin_project::pin_project;

use beserial::Deserialize;
use futures::ready;
use nimiq_block::Block;
use nimiq_block_production::BlockProducer;
use nimiq_blockchain::{AbstractBlockchain, Blockchain};
use nimiq_bls::{KeyPair, SecretKey};
use nimiq_consensus::consensus_agent::ConsensusAgent;
use nimiq_consensus::sync::block_queue::BlockQueueConfig;
use nimiq_consensus::sync::request_component::RequestComponentEvent;
use nimiq_consensus::sync::{block_queue::BlockQueue, request_component::RequestComponent};
use nimiq_database::volatile::VolatileEnvironment;
use nimiq_hash::Blake2bHash;
use nimiq_mempool::{Mempool, MempoolConfig};
use nimiq_network_interface::network::Network;
use nimiq_network_interface::peer::Peer;
use nimiq_network_mock::{MockHub, MockId, MockPeer};
use nimiq_primitives::networks::NetworkId;
use std::marker::PhantomData;
use std::sync::Weak;

/// Secret key of validator. Tests run with `network-primitives/src/genesis/unit-albatross.toml`
const SECRET_KEY: &str =
    "196ffdb1a8acc7cbd76a251aeac0600a1d68b3aba1eba823b5e4dc5dbdcdc730afa752c05ab4f6ef8518384ad514f403c5a088a22b17bf1bc14f8ff8decc2a512c0a200f68d7bdf5a319b30356fe8d1d75ef510aed7a8660968c216c328a0000";

#[pin_project]
#[derive(Debug)]
pub struct MockRequestComponent<P> {
    pub peer_put_into_sync: bool,
    pub tx: mpsc::UnboundedSender<(Blake2bHash, Vec<Blake2bHash>)>,
    #[pin]
    pub rx: mpsc::UnboundedReceiver<Vec<Block>>,
    peer_type: PhantomData<P>,
}

impl<P> MockRequestComponent<P> {
    pub fn new() -> (
        Self,
        mpsc::UnboundedReceiver<(Blake2bHash, Vec<Blake2bHash>)>,
        mpsc::UnboundedSender<Vec<Block>>,
    ) {
        let (tx1, rx1) = mpsc::unbounded();
        let (tx2, rx2) = mpsc::unbounded();

        (
            Self {
                peer_put_into_sync: false,
                tx: tx1,
                rx: rx2,
                peer_type: PhantomData,
            },
            rx1,
            tx2,
        )
    }
}

impl<P: Peer> RequestComponent<P> for MockRequestComponent<P> {
    fn request_missing_blocks(
        &mut self,
        target_block_hash: Blake2bHash,
        locators: Vec<Blake2bHash>,
    ) {
        self.tx.unbounded_send((target_block_hash, locators)).ok(); // ignore error
    }

    fn put_peer_into_sync_mode(&mut self, _peer: Arc<P>) {
        self.peer_put_into_sync = true;
    }

    fn num_peers(&self) -> usize {
        1
    }

    fn peers(&self) -> Vec<Weak<ConsensusAgent<P>>> {
        unimplemented!()
    }
}

impl<P> Default for MockRequestComponent<P> {
    fn default() -> Self {
        Self::new().0
    }
}

impl<P: Peer> Stream for MockRequestComponent<P> {
    type Item = RequestComponentEvent;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<Self::Item>> {
        Poll::Ready(
            ready!(self.project().rx.poll_next(cx)).map(RequestComponentEvent::ReceivedBlocks),
        )
    }
}

#[tokio::test]
async fn send_single_micro_block_to_block_queue() {
    let keypair =
        KeyPair::from(SecretKey::deserialize_from_vec(&hex::decode(SECRET_KEY).unwrap()).unwrap());
    let env = VolatileEnvironment::new(10).unwrap();
    let blockchain = Arc::new(Blockchain::new(env, NetworkId::UnitAlbatross).unwrap());
    let mut hub = MockHub::new();
    let network = Arc::new(hub.new_network());
    let mempool = Mempool::new(Arc::clone(&blockchain), MempoolConfig::default());
    let producer = BlockProducer::new(Arc::clone(&blockchain), Arc::clone(&mempool), keypair);
    let request_component = MockRequestComponent::<MockPeer>::default();
    let (mut tx, rx) = mpsc::channel(32);

    let mut block_queue = BlockQueue::with_block_stream(
        Default::default(),
        Arc::clone(&blockchain),
        Arc::clone(&network),
        request_component,
        rx.boxed(),
    );

    // push one micro block to the queue
    let block =
        Block::Micro(producer.next_micro_block(blockchain.time.now(), 0, None, vec![], vec![0x42]));
    let mock_id = MockId::new(hub.new_address().into());
    tx.send((block, mock_id)).await.unwrap();

    assert_eq!(blockchain.block_number(), 0);

    // run the block_queue one iteration, i.e. until it processed one block
    block_queue.next().await;

    // The produced block is without gap and should go right into the blockchain
    assert_eq!(blockchain.block_number(), 1);
    assert!(block_queue.buffered_blocks().next().is_none());
}

#[tokio::test]
async fn send_two_micro_blocks_out_of_order() {
    let keypair =
        KeyPair::from(SecretKey::deserialize_from_vec(&hex::decode(SECRET_KEY).unwrap()).unwrap());
    let env1 = VolatileEnvironment::new(10).unwrap();
    let env2 = VolatileEnvironment::new(10).unwrap();
    let blockchain1 = Arc::new(Blockchain::new(env1, NetworkId::UnitAlbatross).unwrap());
    let blockchain2 = Arc::new(Blockchain::new(env2, NetworkId::UnitAlbatross).unwrap());
    let mut hub = MockHub::new();
    let network = Arc::new(hub.new_network());
    let mempool = Mempool::new(Arc::clone(&blockchain2), MempoolConfig::default());
    let producer = BlockProducer::new(Arc::clone(&blockchain2), Arc::clone(&mempool), keypair);
    let (request_component, mut mock_ptarc_rx, _mock_ptarc_tx) =
        MockRequestComponent::<MockPeer>::new();
    let (mut tx, rx) = mpsc::channel(32);

    let mut block_queue = BlockQueue::with_block_stream(
        Default::default(),
        Arc::clone(&blockchain1),
        network,
        request_component,
        rx.boxed(),
    );

    let block1 = Block::Micro(producer.next_micro_block(
        blockchain2.time.now(),
        0,
        None,
        vec![],
        vec![0x42],
    ));
    blockchain2.push(block1.clone()).unwrap(); // push it, so the producer actually produces a block at height 2
    let block2 = Block::Micro(producer.next_micro_block(
        blockchain2.time.now() + 1000,
        0,
        None,
        vec![],
        vec![0x42],
    ));

    let mock_id = MockId::new(hub.new_address().into());

    // send block2 first
    tx.send((block2.clone(), mock_id.clone())).await.unwrap();

    assert_eq!(blockchain1.block_number(), 0);

    // run the block_queue one iteration, i.e. until it processed one block
    block_queue.next().await;

    // this block should be buffered now
    assert_eq!(blockchain1.block_number(), 0);
    let blocks = block_queue.buffered_blocks().collect::<Vec<_>>();
    assert_eq!(blocks.len(), 1);
    let (block_number, blocks) = blocks.get(0).unwrap();
    assert_eq!(*block_number, 2);
    assert_eq!(blocks[0], block2);

    // Also we should've received a request to fill this gap
    let (target_block_hash, _locators) = mock_ptarc_rx.next().await.unwrap();
    assert_eq!(target_block_hash, block2.hash());

    // now send block1 to fill the gap
    tx.send((block1.clone(), mock_id)).await.unwrap();

    // run the block_queue one iteration, i.e. until it processed one block
    block_queue.next().await;

    // now both blocks should've been pushed to the blockchain
    assert_eq!(blockchain1.block_number(), 2);
    assert!(block_queue.buffered_blocks().next().is_none());
    assert_eq!(blockchain1.get_block_at(1, true, None).unwrap(), block1);
    assert_eq!(blockchain1.get_block_at(2, true, None).unwrap(), block2);
}

#[tokio::test]
async fn send_block_with_gap_and_respond_to_missing_request() {
    //simple_logger::init_by_env();

    let keypair =
        KeyPair::from(SecretKey::deserialize_from_vec(&hex::decode(SECRET_KEY).unwrap()).unwrap());
    let env1 = VolatileEnvironment::new(10).unwrap();
    let env2 = VolatileEnvironment::new(10).unwrap();
    let blockchain1 = Arc::new(Blockchain::new(env1, NetworkId::UnitAlbatross).unwrap());
    let blockchain2 = Arc::new(Blockchain::new(env2, NetworkId::UnitAlbatross).unwrap());
    let mut hub = MockHub::new();
    let network = Arc::new(hub.new_network_with_address(1));
    let mempool = Mempool::new(Arc::clone(&blockchain2), MempoolConfig::default());
    let producer = BlockProducer::new(Arc::clone(&blockchain2), Arc::clone(&mempool), keypair);
    let (request_component, mut mock_ptarc_rx, mock_ptarc_tx) =
        MockRequestComponent::<MockPeer>::new();
    let (mut tx, rx) = mpsc::channel(32);

    let mut block_queue = BlockQueue::with_block_stream(
        Default::default(),
        Arc::clone(&blockchain1),
        network,
        request_component,
        rx.boxed(),
    );

    let block1 = Block::Micro(producer.next_micro_block(
        blockchain2.time.now(),
        0,
        None,
        vec![],
        vec![0x42],
    ));
    blockchain2.push(block1.clone()).unwrap(); // push it, so the producer actually produces a block at height 2
    let block2 = Block::Micro(producer.next_micro_block(
        blockchain2.time.now() + 1000,
        0,
        None,
        vec![],
        vec![0x42],
    ));

    let mock_id = MockId::new(hub.new_address().into());

    // send block2 first
    tx.send((block2.clone(), mock_id)).await.unwrap();

    assert_eq!(blockchain1.block_number(), 0);

    // run the block_queue one iteration, i.e. until it processed one block
    block_queue.next().await;

    // this block should be buffered now
    assert_eq!(blockchain1.block_number(), 0);
    let blocks = block_queue.buffered_blocks().collect::<Vec<_>>();
    assert_eq!(blocks.len(), 1);
    let (block_number, blocks) = blocks.get(0).unwrap();
    assert_eq!(*block_number, 2);
    assert_eq!(blocks[0], block2);

    // Also we should've received a request to fill this gap
    // TODO: Check block locators
    let (target_block_hash, _locators) = mock_ptarc_rx.next().await.unwrap();
    assert_eq!(target_block_hash, block2.hash());

    // Instead of gossiping the block, we'll answer the missing blocks request
    mock_ptarc_tx.unbounded_send(vec![block1.clone()]).unwrap();

    // run the block_queue one iteration, i.e. until it processed one block
    block_queue.next().await;

    // now both blocks should've been pushed to the blockchain
    assert_eq!(blockchain1.block_number(), 2);
    assert!(block_queue.buffered_blocks().next().is_none());
    assert_eq!(blockchain1.get_block_at(1, true, None).unwrap(), block1);
    assert_eq!(blockchain1.get_block_at(2, true, None).unwrap(), block2);
}

#[tokio::test]
async fn put_peer_back_into_sync_mode() {
    let keypair =
        KeyPair::from(SecretKey::deserialize_from_vec(&hex::decode(SECRET_KEY).unwrap()).unwrap());
    let env1 = VolatileEnvironment::new(10).unwrap();
    let env2 = VolatileEnvironment::new(10).unwrap();
    let blockchain1 = Arc::new(Blockchain::new(env1, NetworkId::UnitAlbatross).unwrap());
    let blockchain2 = Arc::new(Blockchain::new(env2, NetworkId::UnitAlbatross).unwrap());
    let mut hub = MockHub::new();
    let network = Arc::new(hub.new_network_with_address(1));
    let mempool = Mempool::new(Arc::clone(&blockchain2), MempoolConfig::default());
    let producer = BlockProducer::new(Arc::clone(&blockchain2), Arc::clone(&mempool), keypair);
    let (request_component, _, _) = MockRequestComponent::<MockPeer>::new();
    let (mut tx, rx) = mpsc::channel(32);

    let peer_addr = hub.new_address().into();
    let mock_id = MockId::new(peer_addr);
    network.dial_peer(peer_addr).await.unwrap();

    let mut block_queue = BlockQueue::with_block_stream(
        BlockQueueConfig {
            buffer_max: 10,
            window_max: 10,
        },
        Arc::clone(&blockchain1),
        network,
        request_component,
        rx.boxed(),
    );

    for _ in 1..11 {
        let block = Block::Micro(producer.next_micro_block(
            blockchain2.time.now(),
            0,
            None,
            vec![],
            vec![0x42],
        ));
        blockchain2.push(block).unwrap();
    }

    let block = Block::Micro(producer.next_micro_block(
        blockchain2.time.now(),
        0,
        None,
        vec![],
        vec![0x42],
    ));

    tx.send((block, mock_id)).await.unwrap();

    // run the block_queue one iteration, i.e. until it processed one block
    block_queue.next().await;

    assert!(block_queue.request_component.peer_put_into_sync);
}
