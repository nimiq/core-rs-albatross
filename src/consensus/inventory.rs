use parking_lot::RwLock;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Weak};
use std::time::Duration;
use weak_table::PtrWeakHashSet;
use crate::consensus::base::block::{Block, BlockHeader};
use crate::consensus::base::blockchain::{Blockchain, PushResult};
use crate::consensus::base::mempool::Mempool;
use crate::consensus::base::primitive::hash::{Hash, Blake2bHash};
use crate::network::message::{Message, InvVector, InvVectorType, TxMessage};
use crate::network::Peer;
use crate::utils::observer::{Notifier, weak_listener, weak_passthru_listener};
use crate::utils::timers::Timers;
use crate::network::message::GetBlocksMessage;
use crate::network::message::GetBlocksDirection;
use parking_lot::Mutex;
use std::time::Instant;

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum InventoryManagerTimer {
    Request(InvVector)
}

pub struct InventoryManager {
    vectors_to_request: HashMap<InvVector, (Weak<InventoryAgent>, PtrWeakHashSet<Weak<InventoryAgent>>)>,
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

    fn ask_to_request_vector(&mut self, agent: &InventoryAgent, vector: &InvVector) {
        if self.vectors_to_request.contains_key(vector) {
            let record = self.vectors_to_request.get_mut(&vector).unwrap();
            let current_opt = record.0.upgrade();
            if current_opt.is_some() {
                let current = current_opt.unwrap();
                if !current.peer.channel.closed() {
                    let agent_arc = agent.self_weak.read().upgrade().unwrap();
                    if !Arc::ptr_eq(&agent_arc, &current) {
                        record.1.insert(agent_arc);
                    }
                    return;
                }
            }

            record.0 = agent.self_weak.read().clone();
            self.request_vector(agent, vector);
        } else {
            let record = (agent.self_weak.read().clone(), PtrWeakHashSet::new());
            self.vectors_to_request.insert(vector.clone(), record);
            self.request_vector(agent, vector);
        }
    }

    fn request_vector(&mut self, agent: &InventoryAgent, vector: &InvVector) {
        agent.queue_vector(vector.clone());

        let weak = self.self_weak.clone();
        let agent1 = agent.self_weak.read().clone();
        let vector1 = vector.clone();
        self.timers.set_delay(InventoryManagerTimer::Request(vector.clone()), move || {
            let this = upgrade_weak!(weak);
            this.write().note_vector_not_received(&agent1, &vector1);
        }, Self::REQUEST_TIMEOUT);
    }

    fn note_vector_received(&mut self, vector: &InvVector) {
        self.timers.clear_delay(&InventoryManagerTimer::Request(vector.clone()));
        self.vectors_to_request.remove(vector);
    }

    fn note_vector_not_received(&mut self, agent_weak: &Weak<InventoryAgent>, vector: &InvVector) {
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

        let next_agent = next_agent_opt.unwrap().clone();
        record.1.remove(&next_agent);
        record.0 = Arc::downgrade(&next_agent);

        self.request_vector(&next_agent, vector);
    }
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum InventoryEvent {
    NewBlockAnnounced,
    KnownBlockAnnounced(Blake2bHash),
    NewTransactionAnnounced,
    KnownTransactionAnnounced,
    NoNewObjectsAnnounced,
    AllObjectsReceived,
    BlockProcessed(Blake2bHash, PushResult),
    GetBlocksTimeout,
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum InventoryAgentTimer {
    GetDataThrottle,
    GetData,
    GetBlocks,
}

struct InventoryAgentState {
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
}

pub struct InventoryAgent {
    blockchain: Arc<Blockchain<'static>>,
    mempool: Arc<Mempool<'static>>,
    peer: Arc<Peer>,
    inv_mgr: Arc<RwLock<InventoryManager>>,
    state: RwLock<InventoryAgentState>,
    pub notifier: RwLock<Notifier<'static, InventoryEvent>>,
    self_weak: RwLock<Weak<InventoryAgent>>,
    timers: Timers<InventoryAgentTimer>,
    mutex: Mutex<()>,
}

impl InventoryAgent {
    const REQUEST_THROTTLE: Duration = Duration::from_millis(500);
    const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
    const REQUEST_THRESHOLD: usize = 50;
    const REQUEST_VECTORS_MAX: usize = 1000;

    pub fn new(blockchain: Arc<Blockchain<'static>>, mempool: Arc<Mempool<'static>>, inv_mgr: Arc<RwLock<InventoryManager>>, peer: Arc<Peer>) -> Arc<Self> {
        let this = Arc::new(InventoryAgent {
            blockchain,
            mempool,
            peer,
            inv_mgr,
            state: RwLock::new(InventoryAgentState {
                bypass_mgr: false,
                known_objects: HashSet::new(),
                blocks_to_request: VecDeque::new(),
                txs_to_request: VecDeque::new(),
                objects_in_flight: HashSet::new(),
            }),
            notifier: RwLock::new(Notifier::new()),
            self_weak: RwLock::new(Weak::new()),
            timers: Timers::new(),
            mutex: Mutex::new(()),
        });
        Self::init_listeners(&this);
        this
    }

    fn init_listeners(this: &Arc<Self>) {
        *this.self_weak.write() = Arc::downgrade(this);

        let channel = &this.peer.channel;
        let msg_notifier = &channel.msg_notifier;
        msg_notifier.inv.write().register(weak_passthru_listener(
            Arc::downgrade(this),
            |this, vectors: Vec<InvVector>| this.on_inv(vectors)));
        msg_notifier.block.write().register(weak_passthru_listener(
            Arc::downgrade(this),
            |this, block: Block| this.on_block(block)));
        msg_notifier.header.write().register(weak_passthru_listener(
            Arc::downgrade(this),
            |this, header: BlockHeader| this.on_header(header)));
        msg_notifier.tx.write().register(weak_passthru_listener(
            Arc::downgrade(this),
            |this, msg: TxMessage| this.on_tx(msg)));
        msg_notifier.not_found.write().register(weak_passthru_listener(
            Arc::downgrade(this),
            |this, vectors: Vec<InvVector>| this.on_not_found(vectors)));

        let mut close_notifier = channel.close_notifier.write();
        close_notifier.register(weak_listener(
            Arc::downgrade(this),
            |this, _| this.on_close()));
    }

    pub fn get_blocks(&self, locators: Vec<Blake2bHash>, max_results: u16, timeout: Duration) {
        let weak = self.self_weak.read().clone();
        self.timers.set_delay(InventoryAgentTimer::GetBlocks, move || {
            let this = upgrade_weak!(weak);
            this.timers.clear_delay(&InventoryAgentTimer::GetBlocks);
            this.notifier.read().notify(InventoryEvent::GetBlocksTimeout);
        }, timeout);

        self.peer.channel.send(GetBlocksMessage::new(
            locators,
            max_results,
            GetBlocksDirection::Forward,
        )).unwrap();
    }

    fn on_inv(&self, vectors: Vec<InvVector>) {
        let lock = self.mutex.lock();

        // Keep track of the objects the peer knows.
        let mut state = self.state.write();
        for vector in vectors.iter() {
            state.known_objects.insert(vector.clone());
        }

        // XXX Clear get_blocks timeout.
        self.timers.clear_delay(&InventoryAgentTimer::GetBlocks);

        // Check which of the advertised objects we know.
        // Request unknown objects, ignore known ones.
        let num_vectors = vectors.len();
        let mut unknown_blocks = Vec::new();
        let mut unknown_txs = Vec::new();
        for vector in vectors {
            if state.objects_in_flight.contains(&vector) {
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
                        self.notifier.read().notify(InventoryEvent::NewBlockAnnounced);
                    } else {
                        self.notifier.read().notify(InventoryEvent::KnownBlockAnnounced(vector.hash.clone()));
                    }
                }
                InvVectorType::Transaction => {
                    if !self.mempool.contains(&vector.hash) {
                        unknown_txs.push(vector);
                        self.notifier.read().notify(InventoryEvent::NewTransactionAnnounced);
                    } else {
                        self.notifier.read().notify(InventoryEvent::KnownTransactionAnnounced);
                    }
                }
                InvVectorType::Error => () // XXX Why do we have this??
            }
        }

        debug!("[INV] {} vectors, {} new blocks, {} new txs",
               num_vectors, unknown_blocks.len(), unknown_txs.len());

        if !unknown_blocks.is_empty() || !unknown_txs.is_empty() {
            if state.bypass_mgr {
                self.queue_vectors(&mut *state, unknown_blocks, unknown_txs);
            } else {
                // TODO optimize
                let inv_mgr_arc = self.inv_mgr.clone();
                let mut inv_mgr = inv_mgr_arc.write();

                // Give up write lock before notifying.
                drop(state);

                for vector in unknown_blocks {
                    inv_mgr.ask_to_request_vector(self, &vector);
                }
                for vector in unknown_txs {
                    inv_mgr.ask_to_request_vector(self, &vector);
                }
            }
        } else {
            // Give up write lock before notifying.
            drop(state);

            self.notifier.read().notify(InventoryEvent::NoNewObjectsAnnounced);
        }
    }

    fn on_block(&self, block: Block) {
        //let lock = self.mutex.lock();

        let hash = block.header.hash::<Blake2bHash>();
        //debug!("[BLOCK] #{} ({} txs) from {}", block.header.height, block.body.as_ref().unwrap().transactions.len(), self.peer.peer_address());

        // Check if we have requested this block.
        let vector = InvVector::new(InvVectorType::Block, hash);
        if !self.state.read().objects_in_flight.contains(&vector) {
            warn!("Unsolicited block from {} - discarding", self.peer.peer_address());
            return;
        }

        // TODO Reuse already known (verified) transactions from mempool.

        self.inv_mgr.write().note_vector_received(&vector);

        // TODO do this async
        // XXX Debug
        let start = Instant::now();
        let height = block.header.height;
        let num_txs = block.body.as_ref().unwrap().transactions.len();

        let result = self.blockchain.push(block);

        debug!("Block #{} ({} txs) took {}ms to process", height, num_txs, (Instant::now() - start).as_millis());

        self.notifier.read().notify(InventoryEvent::BlockProcessed(vector.hash.clone(), result));

        self.on_object_received(&vector);
    }

    fn on_header(&self, header: BlockHeader) {
        debug!("[HEADER] #{} {}", header.height, header.hash::<Blake2bHash>());
    }

    fn on_tx(&self, msg: TxMessage) {
    }

    fn on_not_found(&self, vectors: Vec<InvVector>) {
        debug!("[NOTFOUND] {} vectors", vectors.len());

        // Remove unknown objects from in-flight list.
        let agent = &*self.self_weak.read();
        for vector in vectors {
            if !self.state.read().objects_in_flight.contains(&vector) {
                continue;
            }

            self.inv_mgr.write().note_vector_not_received(agent, &vector);

            // Mark object as received.
            self.on_object_received(&vector);
        }
    }

    fn on_close(&self) {
        self.timers.clear_all();
    }

    fn queue_vector(&self, vector: InvVector) {
        let mut state = self.state.write();
        match vector.ty {
            InvVectorType::Block => state.blocks_to_request.push_back(vector),
            InvVectorType::Transaction => state.txs_to_request.push_back(vector),
            InvVectorType::Error => () // XXX Get rid of this!
        }
        self.request_vectors_throttled(&mut *state);
    }

    fn queue_vectors(&self, state: &mut InventoryAgentState, block_vectors: Vec<InvVector>, tx_vectors: Vec<InvVector>) {
        state.blocks_to_request.reserve(block_vectors.len());
        for vector in block_vectors {
            state.blocks_to_request.push_back(vector);
        }

        state.txs_to_request.reserve(tx_vectors.len());
        for vector in tx_vectors {
            state.txs_to_request.push_back(vector);
        }

        self.request_vectors_throttled(state);
    }

    fn request_vectors_throttled(&self, state: &mut InventoryAgentState) {
        self.timers.clear_delay(&InventoryAgentTimer::GetDataThrottle);

        if state.blocks_to_request.len() + state.txs_to_request.len() > Self::REQUEST_THRESHOLD {
            self.request_vectors(state);
        } else {
            let weak = self.self_weak.read().clone();
            self.timers.set_delay(InventoryAgentTimer::GetDataThrottle, move || {
                let this = upgrade_weak!(weak);
                let mut state = this.state.write();
                this.request_vectors(&mut *state);
            }, Self::REQUEST_THROTTLE)
        }
    }

    fn request_vectors(&self, state: &mut InventoryAgentState) {
        // Only one request at a time.
        if !state.objects_in_flight.is_empty() {
            return;
        }

        // Don't do anything if there are no objects queued to request.
        if state.blocks_to_request.is_empty() && state.txs_to_request.is_empty() {
            return;
        }

        // Request queued objects from the peer. Only request up to VECTORS_MAX_COUNT objects at a time.
        let num_blocks = state.blocks_to_request.len().min(Self::REQUEST_VECTORS_MAX);
        let num_txs = state.txs_to_request.len().min(Self::REQUEST_VECTORS_MAX - num_blocks);

        let mut vectors = Vec::new();
        for vector in state.blocks_to_request.drain(..num_blocks) {
            state.objects_in_flight.insert(vector.clone());
            vectors.push(vector);
        }
        for vector in state.txs_to_request.drain(..num_txs) {
            state.objects_in_flight.insert(vector.clone());
            vectors.push(vector);
        }

        // Request data from peer.
        // TODO handle result
        self.peer.channel.send(Message::GetData(vectors));

        // Set timeout to detect end of request / missing objects.
        let weak = self.self_weak.read().clone();
        self.timers.set_delay(InventoryAgentTimer::GetData, move || {
            let this = upgrade_weak!(weak);
            this.no_more_data();
        }, Self::REQUEST_TIMEOUT);
    }

    fn on_object_received(&self, vector: &InvVector) {
        self.inv_mgr.write().note_vector_received(vector);

        let mut state = self.state.write();
        if state.objects_in_flight.is_empty() {
            return;
        }

        state.objects_in_flight.remove(vector);

        // Reset request timeout if we expect more objects.
        if !state.objects_in_flight.is_empty() {
            let weak = self.self_weak.read().clone();
            self.timers.reset_delay(InventoryAgentTimer::GetData, move || {
                let this = upgrade_weak!(weak);
                this.no_more_data();
            }, Self::REQUEST_TIMEOUT);
        } else {
            drop(state);
            self.no_more_data();
        }
    }

    fn no_more_data(&self) {
        self.timers.clear_delay(&InventoryAgentTimer::GetData);

        let mut state = self.state.write();

        // TODO optimize
        {
            let inv_mgr_arc = self.inv_mgr.clone();
            let mut inv_mgr = inv_mgr_arc.write();
            let agent = &*self.self_weak.read();
            for vector in state.objects_in_flight.drain() {
                inv_mgr.note_vector_not_received(agent, &vector);
            }
        }

        // TODO objects_that_flew

        // If there are more objects to request, request them.
        if !state.blocks_to_request.is_empty() || !state.txs_to_request.is_empty() {
            self.request_vectors(&mut *state);
        } else {
            // Give up write lock before notifying.
            drop(state);
            self.notifier.read().notify(InventoryEvent::AllObjectsReceived);
        }
    }

    // FIXME Naming
    pub fn bypass_mgr(&self, bypass: bool) {
        self.state.write().bypass_mgr = bypass;
    }

    pub fn is_busy(&self) -> bool {
        !self.state.read().objects_in_flight.is_empty() || self.timers.delay_exists(&InventoryAgentTimer::GetBlocks)
    }
}
