use parking_lot::RwLock;
use std::collections::{HashMap, HashSet, VecDeque};
use std::ptr;
use std::sync::{Arc, Weak};
use std::time::Duration;
use weak_table::PtrWeakHashSet;
use crate::consensus::base::block::{Block, BlockHeader};
use crate::consensus::base::blockchain::{Blockchain, PushResult};
use crate::consensus::base::mempool::Mempool;
use crate::consensus::base::primitive::hash::{Hash, Blake2bHash};
use crate::network::message::{Message, InvVector, InvVectorType, TxMessage};
use crate::network::Peer;
use crate::network::peer_channel::PeerChannelEvent;
use crate::utils::observer::{Notifier, weak_listener, weak_passthru_listener};
use crate::utils::timers::Timers;

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum InventoryManagerTimer {
    Request(InvVector)
}

pub struct InventoryManager {
    vectors_to_request: HashMap<InvVector, (Weak<RwLock<InventoryAgent>>, PtrWeakHashSet<Weak<RwLock<InventoryAgent>>>)>,
    self_weak: Weak<RwLock<InventoryManager>>,
    timers: Timers<InventoryManagerTimer>,
}

impl InventoryManager {
    const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

    pub fn new() -> Arc<RwLock<Self>> {
        let this = Arc::new(RwLock::new(InventoryManager {
            vectors_to_request: HashMap::new(),
            self_weak: Weak::new(),
            timers: Timers::new(),
        }));
        this.write().self_weak = Arc::downgrade(&this);
        this
    }

    fn ask_to_request_vector(&mut self, agent: &mut InventoryAgent, vector: &InvVector) {
        if self.vectors_to_request.contains_key(vector) {
            let record = self.vectors_to_request.get_mut(&vector).unwrap();
            let current_opt = record.0.upgrade();
            if current_opt.is_some() {
                let current = current_opt.unwrap();
                if !current.read().peer.channel.closed() {
                    let agent_arc = agent.self_weak.upgrade().unwrap();
                    if !Arc::ptr_eq(&agent_arc, &current) {
                        record.1.insert(agent_arc);
                    }
                    return;
                }
            }

            record.0 = agent.self_weak.clone();
            self.request_vector(agent, vector);
        } else {
            let record = (agent.self_weak.clone(), PtrWeakHashSet::new());
            self.vectors_to_request.insert(vector.clone(), record);
            self.request_vector(agent, vector);
        }
    }

    fn request_vector(&mut self, agent: &mut InventoryAgent, vector: &InvVector) {
        agent.queue_vector(vector.clone());

        let weak = self.self_weak.clone();
        let agent1 = agent.self_weak.clone();
        let vector1 = vector.clone();
        self.timers.set_delay(InventoryManagerTimer::Request(vector.clone()), move || {
            let this = upgrade_weak!(weak);
            this.write().note_vector_not_received(&agent1, &vector1);
        }, InventoryManager::REQUEST_TIMEOUT);
    }

    fn note_vector_received(&mut self, vector: &InvVector) {
        self.timers.clear_delay(&InventoryManagerTimer::Request(vector.clone()));
        self.vectors_to_request.remove(vector);
    }

    fn note_vector_not_received(&mut self, agent_weak: &Weak<RwLock<InventoryAgent>>, vector: &InvVector) {
        let record_opt = self.vectors_to_request.get_mut(vector);
        if record_opt.is_none() {
            return;
        }

        let record = record_opt.unwrap();
        let current_opt = record.0.upgrade();
        let agent_opt = agent_weak.upgrade();
        if current_opt.is_some() {
            if agent_opt.is_none() {
                return;
            }

            let agent = agent_opt.unwrap();
            let current = current_opt.unwrap();
            if !Arc::ptr_eq(&agent, &current) {
                record.1.remove(&agent);
                return;
            }
        }

        let next_agent_opt = record.1.iter().next();
        if next_agent_opt.is_none() {
            self.vectors_to_request.remove(vector);
            return;
        }

        let next_agent_arc = next_agent_opt.unwrap().clone();
        record.1.remove(&next_agent_arc);
        record.0 = Arc::downgrade(&next_agent_arc);

        let mut next_agent = next_agent_arc.write();
        self.request_vector(&mut next_agent, vector);
    }
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
enum InventoryEvent {
    NewBlockAnnounced,
    KnownBlockAnnounced,
    NewTransactionAnnounced,
    KnownTransactionAnnounced,
    NoNewObjectsAnnounced,
    AllObjectsReceived,
    BlockProcessed(PushResult),
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum InventoryAgentTimer {
    RequestThrottle,
    Request,
}

struct InventoryAgent {
    blockchain: Arc<Blockchain<'static>>,
    mempool: Arc<Mempool<'static>>,
    peer: Arc<Peer>,
    inv_mgr: Arc<RwLock<InventoryManager>>,

    /// Flag to indicate that the agent should request unknown objects immediately
    /// instead of coordinating with the InventoryManager. Used during sync.
    bypass_mgr: bool,

    /// Set of all objects (InvVectors) that we think the remote peer knows.
    known_objects: /*LimitInclusionHashSet*/HashSet<InvVector>,

    /// InvVectors we want to request via getData are collected here and periodically requested.
    blocks_to_request: /*UniqueQueue*/VecDeque<InvVector>,
    txs_to_request: /*ThrottledQueue*/VecDeque<InvVector>,

    /// Objects that are currently being requested from the peer.
    objects_in_flight: HashSet<InvVector>,

    /// Objects that are currently being processed by the blockchain/mempool.
    objects_processing: HashSet<InvVector>,

    pub notifier: Notifier<'static, InventoryEvent>,
    self_weak: Weak<RwLock<InventoryAgent>>,
    timers: Timers<InventoryAgentTimer>,
}

impl InventoryAgent {
    const REQUEST_THROTTLE: Duration = Duration::from_millis(500);
    const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
    const REQUEST_THRESHOLD: usize = 50;
    const REQUEST_VECTORS_MAX: usize = 1000;

    pub fn new(blockchain: Arc<Blockchain<'static>>, mempool: Arc<Mempool<'static>>, inv_mgr: Arc<RwLock<InventoryManager>>, peer: Arc<Peer>) -> Arc<RwLock<Self>> {
        let this = Arc::new(RwLock::new(InventoryAgent {
            blockchain,
            mempool,
            peer,
            inv_mgr,
            bypass_mgr: false,
            known_objects: HashSet::new(),
            blocks_to_request: VecDeque::new(),
            txs_to_request: VecDeque::new(),
            objects_in_flight: HashSet::new(),
            objects_processing: HashSet::new(),
            notifier: Notifier::new(),
            self_weak: Weak::new(),
            timers: Timers::new(),
        }));
        InventoryAgent::init_listeners(&this);
        this
    }

    fn init_listeners(this: &Arc<RwLock<Self>>) {
        this.write().self_weak = Arc::downgrade(this);

        let channel = &this.read().peer.channel;
        let mut msg_notifier = channel.msg_notifier.write();
        msg_notifier.inv.register(weak_passthru_listener(
            Arc::downgrade(this),
            |this, vectors: Vec<InvVector>| this.write().on_inv(vectors)));
        msg_notifier.block.register(weak_passthru_listener(
            Arc::downgrade(this),
            |this, block: Block| this.write().on_block(block)));
        msg_notifier.header.register(weak_passthru_listener(
            Arc::downgrade(this),
            |this, header: BlockHeader| this.write().on_header(header)));
        msg_notifier.tx.register(weak_passthru_listener(
            Arc::downgrade(this),
            |this, msg: TxMessage| this.write().on_tx(msg)));
        msg_notifier.not_found.register(weak_passthru_listener(
            Arc::downgrade(this),
            |this, vectors: Vec<InvVector>| this.write().on_not_found(vectors)));

        let mut close_notifier = channel.close_notifier.write();
        close_notifier.register(weak_listener(
            Arc::downgrade(this),
            |this, _| this.write().on_close()));
    }

    fn on_inv(&mut self, vectors: Vec<InvVector>) {
        // Keep track of the objects the peer knows.
        for vector in vectors.iter() {
            self.known_objects.insert(vector.clone());
        }

        // Check which of the advertised objects we know.
        // Request unknown objects, ignore known ones.
        let num_vectors = vectors.len();
        let mut unknown_blocks = Vec::new();
        let mut unknown_txs = Vec::new();
        for vector in vectors {
            if self.objects_in_flight.contains(&vector)|| self.objects_processing.contains(&vector) {
                continue;
            }

            // TODO Filter out objects that we are not interested in.
            //if (!this._shouldRequestData(vector)) {
            //    continue;
            //}

            match vector.ty {
                InvVectorType::Block => {
                    if !self.blockchain.contains(&vector.hash, true) {
                        unknown_blocks.push(vector);
                        self.notifier.notify(InventoryEvent::NewBlockAnnounced);
                    } else {
                        self.notifier.notify(InventoryEvent::KnownBlockAnnounced);
                    }
                }
                InvVectorType::Transaction => {
                    if !self.mempool.contains(&vector.hash) {
                        unknown_txs.push(vector);
                        self.notifier.notify(InventoryEvent::NewTransactionAnnounced);
                    } else {
                        self.notifier.notify(InventoryEvent::KnownTransactionAnnounced);
                    }
                }
                InvVectorType::Error => () // XXX Why do we have this??
            }
        }

        debug!("[INV] {} vectors, {} new blocks, {} new txs",
               num_vectors, unknown_blocks.len(), unknown_txs.len());

        if !unknown_blocks.is_empty() || !unknown_txs.is_empty() {
            if self.bypass_mgr {
                self.queue_vectors(unknown_blocks, unknown_txs);
            } else {
                // TODO optimize
                let inv_mgr_arc = self.inv_mgr.clone();
                let mut inv_mgr = inv_mgr_arc.write();
                for vector in unknown_blocks {
                    inv_mgr.ask_to_request_vector(self, &vector);
                }
                for vector in unknown_txs {
                    inv_mgr.ask_to_request_vector(self, &vector);
                }
            }
        } else {
            self.notifier.notify(InventoryEvent::NoNewObjectsAnnounced);
        }
    }

    fn on_block(&mut self, block: Block) {
        let hash = block.header.hash::<Blake2bHash>();
        debug!("[BLOCK] #{} {} from", block.header.height, hash);

        // Check if we have requested this block.
        let vector = InvVector::new(InvVectorType::Block, hash);
        if !self.objects_in_flight.contains(&vector) {
            warn!("Unsolicited block from {} - discarding", self.peer.peer_address());
            return;
        }

        // TODO Reuse already known (verified) transactions from mempool.

        // Mark object as received.
        self.on_object_received(&vector);
        self.inv_mgr.write().note_vector_received(&vector);

        // TODO do this async
        let result = self.blockchain.push(block);
        self.notifier.notify(InventoryEvent::BlockProcessed(result));
    }

    fn on_header(&mut self, header: BlockHeader) {
        debug!("[HEADER] #{} {}", header.height, header.hash::<Blake2bHash>());
    }

    fn on_tx(&mut self, msg: TxMessage) {
    }

    fn on_not_found(&mut self, vectors: Vec<InvVector>) {
        debug!("[NOTFOUND] {} vectors", vectors.len());

        // Remove unknown objects from in-flight list.
        for vector in vectors {
            if !self.objects_in_flight.contains(&vector) {
                continue;
            }

            self.inv_mgr.write().note_vector_not_received(&self.self_weak, &vector);

            // Mark object as received.
            self.on_object_received(&vector);
        }
    }

    fn on_close(&mut self) {
        self.timers.clear_all();
    }

    fn queue_vector(&mut self, vector: InvVector) {
        match vector.ty {
            InvVectorType::Block => self.blocks_to_request.push_back(vector),
            InvVectorType::Transaction => self.txs_to_request.push_back(vector),
            InvVectorType::Error => () // XXX Get rid of this!
        }
        self.request_vectors_throttled();
    }

    fn queue_vectors(&mut self, block_vectors: Vec<InvVector>, tx_vectors: Vec<InvVector>) {
        self.blocks_to_request.reserve(block_vectors.len());
        for vector in block_vectors {
            self.blocks_to_request.push_back(vector);
        }

        self.txs_to_request.reserve(tx_vectors.len());
        for vector in tx_vectors {
            self.txs_to_request.push_back(vector);
        }

        self.request_vectors_throttled();
    }

    fn request_vectors_throttled(&mut self) {
        self.timers.clear_delay(&InventoryAgentTimer::RequestThrottle);

        if self.blocks_to_request.len() + self.txs_to_request.len() > InventoryAgent::REQUEST_THRESHOLD {
            self.request_vectors();
        } else {
            let weak = self.self_weak.clone();
            self.timers.set_delay(InventoryAgentTimer::RequestThrottle, move || {
                let this = upgrade_weak!(weak);
                this.write().request_vectors();
            }, InventoryAgent::REQUEST_THROTTLE)
        }
    }

    fn request_vectors(&mut self) {
        // Only one request at a time.
        if !self.objects_in_flight.is_empty() {
            return;
        }

        // Don't do anything if there are no objects queued to request.
        if self.blocks_to_request.is_empty() && self.txs_to_request.is_empty() {
            return;
        }

        // Request queued objects from the peer. Only request up to VECTORS_MAX_COUNT objects at a time.
        let num_blocks = self.blocks_to_request.len().min(InventoryAgent::REQUEST_VECTORS_MAX);
        let num_txs = self.txs_to_request.len().min(InventoryAgent::REQUEST_VECTORS_MAX - num_blocks);

        let mut vectors = Vec::new();
        for vector in self.blocks_to_request.drain(..num_blocks) {
            self.objects_in_flight.insert(vector.clone());
            vectors.push(vector);
        }
        for vector in self.txs_to_request.drain(..num_txs) {
            self.objects_in_flight.insert(vector.clone());
            vectors.push(vector);
        }

        // Request data from peer.
        // TODO handle result
        self.peer.channel.send(Message::GetData(vectors));

        // Set timeout to detect end of request / missing objects.
        let weak = self.self_weak.clone();
        self.timers.set_delay(InventoryAgentTimer::Request, move || {
            let this = upgrade_weak!(weak);
            this.write().no_more_data();
        }, InventoryAgent::REQUEST_TIMEOUT);
    }

    fn on_object_received(&mut self, vector: &InvVector) {
        self.inv_mgr.write().note_vector_received(vector);

        if self.objects_in_flight.is_empty() {
            return;
        }

        self.objects_in_flight.remove(vector);

        // Reset request timeout if we expect more objects.
        if !self.objects_in_flight.is_empty() {
            let weak = self.self_weak.clone();
            self.timers.reset_delay(InventoryAgentTimer::Request, move || {
                let this = upgrade_weak!(weak);
                this.write().no_more_data();
            }, InventoryAgent::REQUEST_TIMEOUT);
        } else {
            self.no_more_data();
        }
    }

    fn no_more_data(&mut self) {
        self.timers.clear_delay(&InventoryAgentTimer::Request);

        // TODO optimize
        let inv_mgr_arc = self.inv_mgr.clone();
        let mut inv_mgr = inv_mgr_arc.write();
        for vector in self.objects_in_flight.drain() {
            inv_mgr.note_vector_not_received(&self.self_weak, &vector);
        }

        // TODO objects_that_flew

        // If there are more objects to request, request them.
        if !self.blocks_to_request.is_empty() || !self.txs_to_request.is_empty() {
            self.request_vectors();
        } else {
            self.notifier.notify(InventoryEvent::AllObjectsReceived);
        }
    }

    // FIXME Naming
    pub fn bypass_mgr(&mut self, bypass: bool) {
        self.bypass_mgr = bypass;
    }
}
