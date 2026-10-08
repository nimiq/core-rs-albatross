//! Re-gossip of announced blocks we already know (`BlockQueue::check_announced_block`).
//!
//! A known main-chain block is only re-gossiped (`MsgAcceptance::Accept`) while it is recent.
//! Peers evict the *propagation source* of a block older than `tolerate_past_max` as
//! `TooFarBehind`, and light/pico clients (or freshly synced nodes) have pruned or never had such
//! blocks, so forwarding a stale one got the honest relay evicted instead of the block's relaying
//! peer.

use std::{
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use futures::{poll, Stream, StreamExt};
use nimiq_block::Block;
use nimiq_blockchain::{BlockProducer, Blockchain, BlockchainConfig};
use nimiq_blockchain_interface::AbstractBlockchain;
use nimiq_blockchain_proxy::BlockchainProxy;
use nimiq_consensus::{
    sync::{
        live::{block_queue::BlockQueue, queue::QueueConfig, BlockLiveSync},
        sync_interface::{LiveSync, MacroSync, MacroSyncReturn},
        syncer::Syncer,
    },
    BlsCache,
};
use nimiq_database::mdbx::MdbxDatabase;
use nimiq_light_blockchain::LightBlockchain;
use nimiq_network_interface::network::{MsgAcceptance, Network};
use nimiq_network_mock::{MockHub, MockId, MockNetwork, MockPeerId};
use nimiq_primitives::{networks::NetworkId, policy::Policy};
use nimiq_test_log::test;
use nimiq_test_utils::blockchain::{produce_macro_blocks, signing_key, voting_key};
use nimiq_utils::time::OffsetTime;
use parking_lot::{Mutex, RwLock};
use tokio::{sync::mpsc, task::yield_now};
use tokio_stream::wrappers::ReceiverStream;

type Announcement = (Block, MockId<MockPeerId>);
type TestSyncer = Syncer<MockNetwork, StubMacroSync, BlockLiveSync<MockNetwork>>;

/// The peer announcing the block. It is a direct peer of the relay only.
const ANNOUNCER: MockPeerId = MockPeerId(1000);

/// Macro sync that never yields a peer; the tests add live-sync peers directly.
struct StubMacroSync;

impl Stream for StubMacroSync {
    type Item = MacroSyncReturn<MockPeerId>;

    fn poll_next(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Poll::Pending
    }
}

impl MacroSync<MockPeerId> for StubMacroSync {
    const MAX_REQUEST_EPOCHS: u16 = 3;

    fn add_peer(&mut self, _peer_id: MockPeerId) {}

    fn drain_peers(&mut self) -> Vec<MockPeerId> {
        vec![]
    }

    fn fallback(&mut self, _peers: Vec<MockPeerId>) {}
}

fn full_blockchain() -> Arc<RwLock<Blockchain>> {
    let env = MdbxDatabase::new_volatile(Default::default()).unwrap();
    Arc::new(RwLock::new(
        Blockchain::new(
            env,
            BlockchainConfig::default(),
            NetworkId::UnitAlbatross,
            Arc::new(OffsetTime::new()),
        )
        .unwrap(),
    ))
}

/// Light clients subscribe to the header topic only, so they see micro blocks without bodies.
fn strip_micro_body(mut block: Block) -> Block {
    if let Block::Micro(ref mut micro) = block {
        micro.body = None;
    }
    block
}

/// A light client that followed `source` up to and including `head`. It only retains the micro
/// blocks of the last batch before its last macro block.
fn light_client_following(
    source: &Arc<RwLock<Blockchain>>,
    head: u32,
) -> Arc<RwLock<LightBlockchain>> {
    let light = Arc::new(RwLock::new(LightBlockchain::new(NetworkId::UnitAlbatross)));
    for height in (Policy::genesis_block_number() + 1)..=head {
        let block = source.read().get_block_at(height, true, None).unwrap();
        LightBlockchain::push(light.upgradable_read(), strip_micro_body(block)).unwrap();
    }
    assert_eq!(light.read().block_number(), head);
    light
}

/// A full node that followed `source` up to and including `head`.
fn full_node_following(source: &Arc<RwLock<Blockchain>>, head: u32) -> Arc<RwLock<Blockchain>> {
    let chain = full_blockchain();
    for height in (Policy::genesis_block_number() + 1)..=head {
        let block = source.read().get_block_at(height, true, None).unwrap();
        Blockchain::push(chain.upgradable_read(), block).unwrap();
    }
    assert_eq!(chain.read().block_number(), head);
    chain
}

/// Wires `BlockQueue` + `BlockLiveSync` + `Syncer` the way `SyncerProxy::new_*` does, with the
/// gossipsub block stream replaced by a channel so the test chooses the propagation source.
fn build_syncer(
    hub: &mut MockHub,
    blockchain: BlockchainProxy,
    include_body: bool,
) -> (TestSyncer, Arc<MockNetwork>, mpsc::Sender<Announcement>) {
    let network = Arc::new(hub.new_network());
    let (tx, rx) = mpsc::channel(16);
    let config = QueueConfig {
        include_body,
        ..Default::default()
    };
    let queue = BlockQueue::with_gossipsub_block_stream(
        blockchain.clone(),
        Arc::clone(&network),
        ReceiverStream::new(rx).boxed(),
        config,
    );
    let live_sync = BlockLiveSync::with_queue(
        blockchain.clone(),
        Arc::clone(&network),
        queue,
        Arc::new(Mutex::new(BlsCache::new_test())),
    );
    let syncer = Syncer::new(blockchain, Arc::clone(&network), live_sync, StubMacroSync);
    (syncer, network, tx)
}

/// Lets the queue's background task run and drains the syncer.
async fn pump(syncer: &mut TestSyncer) {
    for _ in 0..64 {
        yield_now().await;
        let _ = poll!(syncer.next());
    }
}

/// The relay's gossipsub verdict on the header message.
fn header_verdict(network: &MockNetwork) -> Option<MsgAcceptance> {
    network
        .take_validation_results()
        .into_iter()
        .find(|(topic, _, _)| *topic == "block-header")
        .map(|(_, _, acceptance)| acceptance)
}

/// A relay and a victim following it, the victim having the relay as its only live-sync peer.
struct Pair {
    relay: TestSyncer,
    relay_net: Arc<MockNetwork>,
    relay_tx: mpsc::Sender<Announcement>,
    victim: TestSyncer,
    victim_tx: mpsc::Sender<Announcement>,
}

impl Pair {
    fn new(relay_chain: BlockchainProxy, victim_chain: BlockchainProxy) -> Self {
        let mut hub = MockHub::new();
        let (mut relay, relay_net, relay_tx) = build_syncer(&mut hub, relay_chain, true);
        // The queue only looks at gossip while it has a live-sync peer.
        relay.live_sync.add_peer(MockPeerId(2000));

        let (mut victim, _victim_net, victim_tx) = build_syncer(&mut hub, victim_chain, false);
        victim.live_sync.add_peer(relay_net.get_local_peer_id());
        assert_eq!(victim.live_sync.num_peers(), 1);

        Self {
            relay,
            relay_net,
            relay_tx,
            victim,
            victim_tx,
        }
    }

    /// The announcer publishes `block` to the relay. What gossipsub then does with the relay's
    /// verdict: `Accept` forwards the message to the mesh with the relay as propagation source
    /// (light clients receive the header only), anything else drops it. Returns the relay's
    /// verdict and whether the victim still has the relay as its live-sync peer.
    async fn announce(&mut self, block: Block) -> (Option<MsgAcceptance>, bool) {
        self.relay_tx
            .send((block.clone(), MockId::new(ANNOUNCER)))
            .await
            .unwrap();
        pump(&mut self.relay).await;

        let verdict = header_verdict(&self.relay_net);
        if matches!(verdict, Some(MsgAcceptance::Accept)) {
            self.victim_tx
                .send((
                    strip_micro_body(block),
                    MockId::new(self.relay_net.get_local_peer_id()),
                ))
                .await
                .unwrap();
            pump(&mut self.victim).await;
        }
        (verdict, self.victim.live_sync.num_peers() == 1)
    }
}

#[test(tokio::test)]
async fn stale_known_block_is_not_regossiped() {
    let genesis = Policy::genesis_block_number();

    // Honest full node two batches past genesis. It keeps every block of the current epoch.
    let relay_chain = full_blockchain();
    let producer = BlockProducer::new(signing_key(), voting_key());
    produce_macro_blocks(&producer, &relay_chain, 2);
    let head = relay_chain.read().block_number();

    // A genuine micro block on everyone's main chain, more than one batch old.
    let stale = relay_chain
        .read()
        .get_block_at(genesis + 3, true, None)
        .unwrap();
    assert!(stale.is_micro());
    assert!(stale.block_number() < head - Policy::blocks_per_batch());

    // Light/pico victim at the same head. It has pruned `stale`.
    let victim_chain = light_client_following(&relay_chain, head);
    assert!(victim_chain.read().get_block(&stale.hash(), false).is_err());

    let mut pair = Pair::new(
        BlockchainProxy::from(Arc::clone(&relay_chain)),
        BlockchainProxy::from(Arc::clone(&victim_chain)),
    );
    let (verdict, relay_kept) = pair.announce(stale).await;

    assert!(
        matches!(verdict, Some(MsgAcceptance::Ignore)),
        "{verdict:?}"
    );
    assert!(
        relay_kept,
        "honest relay evicted from the victim's live sync"
    );
}

#[test(tokio::test)]
async fn relay_one_block_behind_is_kept_across_macro_boundary() {
    let batch = Policy::blocks_per_batch();

    // Canonical chain, two batches. The victim has just pushed the second macro block.
    let source = full_blockchain();
    let producer = BlockProducer::new(signing_key(), voting_key());
    produce_macro_blocks(&producer, &source, 2);
    let victim_head = source.read().block_number();
    assert!(Policy::is_macro_block_at(victim_head));
    // The relay has not seen that macro block yet: the ordinary skew while a block propagates.
    let relay_head = victim_head - 1;

    let relay_chain = full_node_following(&source, relay_head);
    let victim_chain = light_client_following(&source, victim_head);

    // The block exactly `tolerate_past_max` behind the relay's head: already pruned by the victim
    // (light clients prune to one batch behind each macro block they push), hence evictable.
    let boundary = source
        .read()
        .get_block_at(relay_head - batch, true, None)
        .unwrap();
    assert!(boundary.is_micro());
    assert!(victim_chain
        .read()
        .get_block(&boundary.hash(), false)
        .is_err());

    let mut pair = Pair::new(
        BlockchainProxy::from(Arc::clone(&relay_chain)),
        BlockchainProxy::from(Arc::clone(&victim_chain)),
    );
    let (verdict, relay_kept) = pair.announce(boundary).await;

    assert!(
        matches!(verdict, Some(MsgAcceptance::Ignore)),
        "{verdict:?}"
    );
    assert!(
        relay_kept,
        "relay one block behind the victim evicted after forwarding the boundary block"
    );
}

#[test(tokio::test)]
async fn recent_known_block_is_regossiped() {
    // Honest full node two batches past genesis; the victim follows it to the same head.
    let relay_chain = full_blockchain();
    let producer = BlockProducer::new(signing_key(), voting_key());
    produce_macro_blocks(&producer, &relay_chain, 2);
    let head = relay_chain.read().block_number();
    let victim_chain = light_client_following(&relay_chain, head);

    // A known micro block from the current batch, e.g. the gossip copy of a block we already
    // fetched through a missing-blocks request. Still forwarded, and nobody evicts anybody.
    let recent = relay_chain
        .read()
        .get_block_at(head - 1, true, None)
        .unwrap();
    assert!(recent.is_micro());
    assert!(victim_chain.read().get_block(&recent.hash(), false).is_ok());

    let mut pair = Pair::new(
        BlockchainProxy::from(Arc::clone(&relay_chain)),
        BlockchainProxy::from(Arc::clone(&victim_chain)),
    );
    let (verdict, relay_kept) = pair.announce(recent).await;

    assert!(
        matches!(verdict, Some(MsgAcceptance::Accept)),
        "{verdict:?}"
    );
    assert!(relay_kept);
}
