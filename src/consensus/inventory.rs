use parking_lot::RwLock;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use crate::consensus::base::blockchain::Blockchain;
use crate::consensus::base::mempool::Mempool;
use crate::network::Peer;
use crate::network::message::InvVector;
use crate::network::peer_channel::{PeerChannelEvent};
use crate::network::message::Message;
use crate::utils::observer::weak_passthru_listener;
use crate::consensus::base::block::Block;
use crate::consensus::base::block::BlockHeader;
use crate::network::message::TxMessage;
use crate::utils::observer::weak_listener;

pub struct InventoryManager {
    vectors_to_request: HashMap<InvVector, (Arc<InventoryAgent>, HashSet<Arc<InventoryAgent>>)>

}

impl InventoryManager {
    pub fn new() -> Self {
        InventoryManager {
            vectors_to_request: HashMap::new()
        }
    }


}


struct InventoryAgent {
    blockchain: Arc<Blockchain<'static>>,
    mempool: Arc<Mempool<'static>>,
    peer: Arc<Peer>,

    /// Set of all objects (InvVectors) that we think the remote peer knows.
    known_objects: /*LimitInclusionHashSet*/HashSet<InvVector>,

    /// InvVectors we want to request via getData are collected here and periodically requested.
    blocks_to_request: /*UniqueQueue*/VecDeque<InvVector>,
    txs_to_request: /*ThrottledQueue*/VecDeque<InvVector>,

    /// Objects that are currently being requested from the peer.
    objects_in_flight: HashSet<InvVector>,

    /// Objects that are currently being processed by the blockchain/mempool.
    objects_processing: HashSet<InvVector>,


}

impl InventoryAgent {
    pub fn new(blockchain: Arc<Blockchain<'static>>, mempool: Arc<Mempool<'static>>, peer: Arc<Peer>) -> Arc<RwLock<Self>> {
        let agent = Arc::new(RwLock::new(InventoryAgent {
            blockchain,
            mempool,
            peer,
            known_objects: HashSet::new(),
            blocks_to_request: VecDeque::new(),
            txs_to_request: VecDeque::new(),
            objects_in_flight: HashSet::new(),
            objects_processing: HashSet::new()
        }));
        InventoryAgent::init_listeners(&agent);
        agent
    }

    fn init_listeners(agent: &Arc<RwLock<Self>>) {
        let channel = &agent.read().peer.channel;
        let mut msg_notifier = channel.msg_notifier.write();
        msg_notifier.inv.register(weak_passthru_listener(
            Arc::downgrade(agent),
            |agent, vectors: Vec<InvVector>| agent.write().on_inv(vectors)));
        msg_notifier.block.register(weak_passthru_listener(
            Arc::downgrade(agent),
            |agent, block: Block| agent.write().on_block(block)));
        msg_notifier.header.register(weak_passthru_listener(
            Arc::downgrade(agent),
            |agent, header: BlockHeader| agent.write().on_header(header)));
        msg_notifier.tx.register(weak_passthru_listener(
            Arc::downgrade(agent),
            |agent, msg: TxMessage| agent.write().on_tx(msg)));
        msg_notifier.not_found.register(weak_passthru_listener(
            Arc::downgrade(agent),
            |agent, vectors: Vec<InvVector>| agent.write().on_not_found(vectors)));

        let mut close_notifier = channel.close_notifier.write();
        close_notifier.register(weak_listener(
            Arc::downgrade(agent),
            |agent, _| agent.write().on_close()));
    }

    fn on_close(&mut self) {

    }

    fn on_inv(&mut self, vectors: Vec<InvVector>) {
        // Keep track of the objects the peer knows.
        for vector in vectors.iter() {
            self.known_objects.insert(vector.clone());
        }

        // Check which of the advertised objects we know.
        // Request unknown objects, ignore known ones.
        //let unknown_blocks = Vec::new();
        //let unknown_txs = Vec::new();
        for vector in vectors.iter() {
            if self.objects_in_flight.contains(vector)|| self.objects_processing.contains(vector) {
                continue;
            }

            // TODO Filter out objects that we are not interested in.
            //if (!this._shouldRequestData(vector)) {
            //    continue;
            //}


        }
    }

    fn on_block(&mut self, block: Block) {

    }

    fn on_header(&mut self, header: BlockHeader) {

    }

    fn on_tx(&mut self, msg: TxMessage) {

    }

    fn on_not_found(&mut self, vectors: Vec<InvVector>) {

    }
}
