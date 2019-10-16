use std::cmp;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

use parking_lot::{Mutex, RwLock};
use weak_table::PtrWeakHashSet;

use beserial::Serialize;
use block_base::{Block, BlockError, BlockHeader};
use blockchain_base::{AbstractBlockchain, Direction, PushError, PushResult};
use collections::{LimitHashSet, UniqueLinkedList};
use collections::queue::Queue;
use hash::{Blake2bHash, Hash};
use mempool::{Mempool, ReturnCode};
use network::connection::close_type::CloseType;
use network::Peer;
use network_messages::{
    EpochTransactionsMessage,
    GetBlocksDirection,
    GetBlocksMessage,
    InvVector,
    InvVectorType,
    Message,
    MessageAdapter,
    TxMessage,
};
use network_primitives::networks::NetworkInfo;
use network_primitives::subscription::Subscription;
use transaction::Transaction;
use utils::{
    self,
    mutable_once::MutableOnce,
    observer::{Notifier, weak_listener, weak_passthru_listener},
    timers::Timers,
};
use utils::rate_limit::RateLimit;
use utils::throttled_queue::ThrottledQueue;

use crate::consensus_agent::sync::{SyncEvent, SyncProtocol};
use crate::ConsensusProtocol;

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum InventoryManagerTimer {
    Request(InvVector)
}

type VectorsToRequest<P> = HashMap<InvVector, (Weak<InventoryAgent<P>>, PtrWeakHashSet<Weak<InventoryAgent<P>>>)>;

pub struct InventoryManager<P: ConsensusProtocol + 'static> {
    vectors_to_request: VectorsToRequest<P>,
    self_weak: Weak<RwLock<InventoryManager<P>>>,
    timers: Timers<InventoryManagerTimer>,
}

impl<P: ConsensusProtocol + 'static> InventoryManager<P> {
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

    fn ask_to_request_vector(&mut self, agent: &InventoryAgent<P>, vector: &InvVector) {
        if self.vectors_to_request.contains_key(vector) {
            let record = self.vectors_to_request.get_mut(&vector).unwrap();
            let current_opt = record.0.upgrade();
            if let Some(current) = current_opt {
                if !current.peer.channel.closed() {
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

    fn request_vector(&mut self, agent: &InventoryAgent<P>, vector: &InvVector) {
        agent.queue_vector(vector.clone());

        let weak = self.self_weak.clone();
        let agent1 = agent.self_weak.clone();
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

    fn note_vector_not_received(&mut self, agent_weak: &Weak<InventoryAgent<P>>, vector: &InvVector) {
        self.timers.clear_delay(&InventoryManagerTimer::Request(vector.clone()));
        let record_opt = self.vectors_to_request.get_mut(vector);
        if record_opt.is_none() {
            return;
        }

        let record = record_opt.unwrap();
        let current_opt = record.0.upgrade();
        let agent_opt = agent_weak.upgrade();
        if let Some(current) = current_opt {
            if agent_opt.is_none() {
                return;
            }

            let agent = agent_opt.unwrap();
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

#[derive(Debug, PartialEq, Eq)]
pub enum InventoryEvent<BE: BlockError> {
    NewBlockAnnounced,
    KnownBlockAnnounced(Blake2bHash),
    NewTransactionAnnounced,
    KnownTransactionAnnounced,
    NoNewObjectsAnnounced,
    AllObjectsReceived,
    BlockProcessed(Blake2bHash, Result<PushResult, PushError<BE>>),
    TransactionProcessed(Blake2bHash, ReturnCode),
    GetBlocksTimeout,
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum InventoryAgentTimer {
    GetDataThrottle,
    GetData,
    GetBlocks,
    TxInvVectors,
    FreeTxInvVectors,
}

#[derive(Debug, Clone)]
struct FreeTransactionVector {
    vector: InvVector,
    serialized_size: usize,
}

impl FreeTransactionVector {
    fn from_vector(vector: &InvVector, serialized_size: usize) -> Self {
        FreeTransactionVector {
            vector: vector.clone(),
            serialized_size,
        }
    }
}

impl PartialEq for FreeTransactionVector {
    fn eq(&self, other: &FreeTransactionVector) -> bool {
        self.vector == other.vector
    }
}

impl Eq for FreeTransactionVector {}

impl std::hash::Hash for FreeTransactionVector {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        std::hash::Hash::hash(&self.vector, state);
    }
}

impl From<FreeTransactionVector> for InvVector {
    fn from(vector: FreeTransactionVector) -> Self {
        vector.vector
    }
}

struct InventoryAgentState {
    /// Flag to indicate that the agent should request unknown objects immediately
    /// instead of coordinating with the InventoryManager. Used during sync.
    bypass_mgr: bool,

    /// Set of all objects (InvVectors) that we think the remote peer knows.
    known_objects: LimitHashSet<InvVector>,

    /// InvVectors we want to request via getData are collected here and periodically requested.
    blocks_to_request: UniqueLinkedList<InvVector>,
    txs_to_request: ThrottledQueue<InvVector>,

    /// Queue of transaction inv vectors waiting to be sent out.
    waiting_tx_inv_vectors: ThrottledQueue<InvVector>,
    /// Queue of "free" transaction inv vectors waiting to be sent out.
    waiting_free_tx_inv_vectors: ThrottledQueue<FreeTransactionVector>,

    /// Objects that are currently being requested from the peer.
    objects_in_flight: HashSet<InvVector>,

    /// All objects that were requested from the peer but not received yet.
    objects_that_flew: HashSet<InvVector>,

    /// The rate limit for getblocks messages.
    get_blocks_limit: RateLimit,

    /// A Subscription object specifying which objects should be announced to the peer.
    remote_subscription: Subscription,

    local_subscription: Subscription,

    last_subscription_change: Instant,
}

pub struct InventoryAgent<P: ConsensusProtocol + 'static> {
    blockchain: Arc<P::Blockchain>,
    mempool: Arc<Mempool<'static, P::Blockchain>>,
    peer: Arc<Peer>,
    inv_mgr: Arc<RwLock<InventoryManager<P>>>,
    sync_protocol: Arc<P::SyncProtocol>,
    state: RwLock<InventoryAgentState>,
    pub notifier: RwLock<Notifier<'static, InventoryEvent<<<P::Blockchain as AbstractBlockchain<'static>>::Block as Block>::Error>>>,
    self_weak: MutableOnce<Weak<InventoryAgent<P>>>,
    timers: Timers<InventoryAgentTimer>,
    mutex: Mutex<()>,
}

impl<P: ConsensusProtocol + 'static> InventoryAgent<P> {
    /// Time to wait after the last received inv message before sending get-data.
    const REQUEST_THROTTLE: Duration = Duration::from_millis(500);
    /// Maximum time to wait after sending out get-data or receiving the last object for this request.
    const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
    /// Number of InvVectors in invToRequest pool to automatically trigger a get-data request.
    const REQUEST_THRESHOLD: usize = 50;
    const REQUEST_VECTORS_MAX: usize = 1000;
    const GET_BLOCKS_VECTORS_MAX: u32 = 500;
    const KNOWN_OBJECTS_COUNT_MAX: usize = 40000;
    /// Time interval to wait between sending out transactions.
    const TRANSACTION_RELAY_INTERVAL: Duration = Duration::from_millis(5000);
    const TRANSACTIONS_AT_ONCE: usize = 100;
    const TRANSACTIONS_PER_SECOND: usize = 10;
    /// Time interval to wait between sending out "free" transactions.
    const FREE_TRANSACTION_RELAY_INTERVAL: Duration = Duration::from_millis(6000);
    const FREE_TRANSACTIONS_AT_ONCE: usize = 10;
    const FREE_TRANSACTIONS_PER_SECOND: usize = 1;
    /// Soft limit for the total size (bytes) of free transactions per relay interval.
    const FREE_TRANSACTION_SIZE_PER_INTERVAL: usize = 15000; // ~100 legacy transactions
    const TRANSACTION_THROTTLE: Duration = Duration::from_millis(1000);
    const REQUEST_TRANSACTIONS_WAITING_MAX: usize = 5000;
    const GET_BLOCKS_RATE_LIMIT: usize = 30; // per minute
    /// Time {ms} to wait between sending full inv vectors of transactions during Mempool request
    const MEMPOOL_THROTTLE: Duration = Duration::from_millis(1000); // 1 second
    const MEMPOOL_ENTRIES_MAX: usize = 10_000;
    /// Minimum fee per byte (sat/byte) such that a transaction is not considered free.
    const TRANSACTION_RELAY_FEE_MIN: u64 = 1;

    const SUBSCRIPTION_CHANGE_GRACE_PERIOD: Duration = Duration::from_secs(2);

    pub fn new(blockchain: Arc<P::Blockchain>, mempool: Arc<Mempool<'static, P::Blockchain>>, inv_mgr: Arc<RwLock<InventoryManager<P>>>, peer: Arc<Peer>, sync_agent: Arc<P::SyncProtocol>) -> Arc<Self> {
        let this = Arc::new(InventoryAgent {
            blockchain,
            mempool,
            peer,
            inv_mgr,
            sync_protocol: sync_agent,
            state: RwLock::new(InventoryAgentState {
                bypass_mgr: false,
                known_objects: LimitHashSet::new(Self::KNOWN_OBJECTS_COUNT_MAX),
                blocks_to_request: UniqueLinkedList::new(),
                txs_to_request: ThrottledQueue::new(
                    Self::TRANSACTIONS_AT_ONCE + Self::FREE_TRANSACTIONS_AT_ONCE,
                    Self::TRANSACTION_THROTTLE,
                    Self::TRANSACTIONS_PER_SECOND + Self::FREE_TRANSACTIONS_PER_SECOND,
                    Some(Self::REQUEST_TRANSACTIONS_WAITING_MAX),
                ),

                waiting_tx_inv_vectors: ThrottledQueue::new(
                    Self::TRANSACTIONS_AT_ONCE,
                    Self::TRANSACTION_THROTTLE,
                    Self::TRANSACTIONS_PER_SECOND,
                    Some(Self::REQUEST_TRANSACTIONS_WAITING_MAX),
                ),
                waiting_free_tx_inv_vectors: ThrottledQueue::new(
                    Self::FREE_TRANSACTIONS_AT_ONCE,
                    Self::TRANSACTION_THROTTLE,
                    Self::FREE_TRANSACTIONS_PER_SECOND,
                    Some(Self::REQUEST_TRANSACTIONS_WAITING_MAX),
                ),

                objects_in_flight: HashSet::new(),

                objects_that_flew: HashSet::new(),

                get_blocks_limit: RateLimit::new_per_minute(Self::GET_BLOCKS_RATE_LIMIT),

                // Initially, we don't announce anything to the peer until it tells us otherwise.
                remote_subscription: Subscription::None,

                local_subscription: Subscription::None,

                last_subscription_change: Instant::now(),
            }),
            notifier: RwLock::new(Notifier::new()),
            self_weak: MutableOnce::new(Weak::new()),
            timers: Timers::new(),
            mutex: Mutex::new(()),
        });
        Self::init_listeners(&this);
        this
    }

    fn init_listeners(this: &Arc<Self>) {
        unsafe { this.self_weak.replace(Arc::downgrade(this)) };

        let channel = &this.peer.channel;
        let msg_notifier = &channel.msg_notifier;
        msg_notifier.inv.write().register(weak_passthru_listener(
            Arc::downgrade(this),
            |this, vectors: Vec<InvVector>| this.on_inv(vectors)));
        P::MessageAdapter::register_block_listener(msg_notifier, weak_passthru_listener(
            Arc::downgrade(this),
            |this, block| this.on_block(block)));
        P::MessageAdapter::register_header_listener(msg_notifier, weak_passthru_listener(
            Arc::downgrade(this),
            |this, header| this.on_header(header)));
        msg_notifier.tx.write().register(weak_passthru_listener(
            Arc::downgrade(this),
            |this, msg: TxMessage| this.on_tx(msg)));
        msg_notifier.not_found.write().register(weak_passthru_listener(
            Arc::downgrade(this),
            |this, vectors: Vec<InvVector>| this.on_not_found(vectors)));

        msg_notifier.get_blocks.write().register(weak_passthru_listener(
            Arc::downgrade(this),
            |this, msg: GetBlocksMessage| this.on_get_blocks(msg)));
        msg_notifier.get_data.write().register(weak_passthru_listener(
            Arc::downgrade(this),
            |this, vectors: Vec<InvVector>| this.on_get_data(vectors)));
        msg_notifier.get_header.write().register(weak_passthru_listener(
            Arc::downgrade(this),
            |this, vectors: Vec<InvVector>| this.on_get_header(vectors)));
        msg_notifier.mempool.write().register(weak_passthru_listener(
            Arc::downgrade(this),
            |this, _ | this.on_mempool()));
        msg_notifier.get_macro_blocks.write().register(weak_passthru_listener(
            Arc::downgrade(this),
            |this, msg: GetBlocksMessage| this.on_get_macro_blocks(msg)));
        msg_notifier.epoch_transactions.write().register(weak_passthru_listener(
            Arc::downgrade(this),
            |this, msg: EpochTransactionsMessage| this.on_epoch_transactions(msg)));

        msg_notifier.subscribe.write().register(weak_passthru_listener(
            Arc::downgrade(this),
            |this, subscription: Subscription| this.on_subscribe(subscription)));

        this.sync_protocol.register_listener(weak_passthru_listener(Arc::downgrade(this),
        |this, event| {
            match event {
                SyncEvent::BlockProcessed(hash, result) => this.notifier.read().notify(InventoryEvent::BlockProcessed(hash, result)),
            }
        }));

        let mut close_notifier = channel.close_notifier.write();
        close_notifier.register(weak_listener(
            Arc::downgrade(this),
            |this, _| this.on_close()));

        let weak = Arc::downgrade(this);
        this.timers.set_interval(InventoryAgentTimer::TxInvVectors, move || {
            let this = upgrade_weak!(weak);
            this.send_waiting_tx_inv_vectors();
        }, Self::TRANSACTION_RELAY_INTERVAL);
        let weak = Arc::downgrade(this);
        this.timers.set_interval(InventoryAgentTimer::FreeTxInvVectors, move || {
            let this = upgrade_weak!(weak);
            this.send_waiting_free_tx_inv_vectors();
        }, Self::FREE_TRANSACTION_RELAY_INTERVAL);
    }

    pub fn get_blocks(&self, locators: Vec<Blake2bHash>, max_results: u16, timeout: Duration) {
        let weak = self.self_weak.clone();
        self.timers.set_delay(InventoryAgentTimer::GetBlocks, move || {
            let this = upgrade_weak!(weak);
            this.timers.clear_delay(&InventoryAgentTimer::GetBlocks);
            this.notifier.read().notify(InventoryEvent::GetBlocksTimeout);
        }, timeout);

        self.sync_protocol.request_blocks(locators, max_results);
    }

    pub fn mempool(&self) {
        self.peer.channel.send_or_close(Message::Mempool);
    }

    pub fn subscribe(&self, subscription: Subscription) {
        let mut state = self.state.write();
        state.local_subscription = subscription.clone();
        state.last_subscription_change = Instant::now();
        self.peer.channel.send_or_close(Message::Subscribe(Box::new(subscription)));
    }

    fn should_request_data(&self, vector: &InvVector) -> bool {
        // Ignore block announcements from nano clients as they will ignore our getData requests anyways (they only know headers).
        // Also don't request transactions that the mempool has filtered.
        match vector.ty {
            InvVectorType::Block => !self.peer.peer_address().services.is_nano_node(),
            InvVectorType::Transaction => !self.mempool.is_filtered(&vector.hash),
            _ => false,
        }
    }

    fn on_subscribe(&self, subscription: Subscription) {
        self.state.write().remote_subscription = subscription;
    }

    fn on_inv(&self, vectors: Vec<InvVector>) {
        let _lock = self.mutex.lock();

        // Keep track of the objects the peer knows.
        let mut state = self.state.write();
        for vector in vectors.iter() {
            state.known_objects.insert(vector.clone());
            state.waiting_tx_inv_vectors.remove(vector);
            // Serialized size does not matter here due to the implementation of Hash and Eq.
            state.waiting_free_tx_inv_vectors.remove(&FreeTransactionVector::from_vector(vector, 0));
        }

        // XXX Clear get_blocks timeout.
        self.timers.clear_delay(&InventoryAgentTimer::GetBlocks);

        // Check which of the advertised objects we know.
        // Request unknown objects, ignore known ones.
        let num_vectors = vectors.len();
        let mut unknown_blocks = Vec::new();
        let mut unknown_txs = Vec::new();
        let vectors: Vec<InvVector> = vectors.into_iter().filter(|vector| {
            !state.objects_in_flight.contains(&vector) && self.should_request_data(&vector)
        }).collect();
        // Give up state write lock.
        drop(state);
        for vector in vectors {
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

        trace!("[INV] {} vectors, {} new blocks, {} new txs from {}",
               num_vectors, unknown_blocks.len(), unknown_txs.len(), self.peer.peer_address());

        // Re-take state write lock.
        let mut state = self.state.write();
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

            self.sync_protocol.on_no_new_objects_announced();
            self.notifier.read().notify(InventoryEvent::NoNewObjectsAnnounced);
        }
    }

    fn on_block(&self, mut block: <P::Blockchain as AbstractBlockchain<'static>>::Block) {
        //let lock = self.mutex.lock();

        let hash = block.hash();
        trace!("[BLOCK] #{} ({} txs) from {}", block.height(), block.transactions().map(|txs| txs.len()).unwrap_or(0), self.peer.peer_address());

        // Check if we have requested this block.
        let vector = InvVector::from_block_hash(hash);
        let state = self.state.read();
        if !state.objects_in_flight.contains(&vector) && !state.objects_that_flew.contains(&vector) {
            warn!("Unsolicited block from {} - discarding", self.peer.peer_address());
            return;
        }
        // Give up read lock before notifying.
        drop(state);

        // Use already known (verified) transactions from mempool to set validity.
        if let Some(ref mut transactions) = block.transactions_mut() {
            for i in 0..transactions.len() {
                if let Some(mempool_tx) = self.mempool.get_transaction(&transactions[i].hash()) {
                    transactions[i].check_set_valid(&mempool_tx);
                }
            }
        }

        self.inv_mgr.write().note_vector_received(&vector);

        // Process block.
        self.sync_protocol.on_block(block);

        // Mark object as received.
        self.on_object_received(&vector);
    }

    fn on_header(&self, header: <<P::Blockchain as AbstractBlockchain<'static>>::Block as Block>::Header) {
        trace!("[HEADER] #{} {}", header.height(), header.hash());
        warn!("Unsolicited header message received from {}, discarding", self.peer.peer_address());
    }

    fn on_tx(&self, msg: TxMessage) {
        let hash = msg.transaction.hash::<Blake2bHash>();
        trace!("[TX] from {} value {} fee {}", msg.transaction.sender, msg.transaction.value, msg.transaction.fee);

        // Check if we have requested this transaction.
        let vector = InvVector::new(InvVectorType::Transaction, hash.clone());
        let state = self.state.read();
        if !state.objects_in_flight.contains(&vector) && !state.objects_that_flew.contains(&vector) {
            warn!("Unsolicited transaction from {} - discarding", self.peer.peer_address());
            return;
        }
        // Give up read lock before notifying.
        drop(state);

        self.inv_mgr.write().note_vector_received(&vector);

        // Mark object as received.
        self.on_object_received(&vector);

        // Check whether we subscribed for this transaction.
        let state = self.state.read();
        if state.local_subscription.matches_transaction(&msg.transaction) {
            // Give up read lock before pushing transaction.
            drop(state);

            let result = self.mempool.push_transaction(msg.transaction);
            self.notifier.read().notify(InventoryEvent::TransactionProcessed(vector.hash.clone(), result));
        } else if state.last_subscription_change.elapsed() > Self::SUBSCRIPTION_CHANGE_GRACE_PERIOD {
            // Give up read lock.
            drop(state);

            warn!("We're not subscribed to this transaction from {} - discarding and closing the channel", self.peer.peer_address());
            self.peer.channel.close(CloseType::ReceivedTransactionNotMatchingOurSubscription);
        }
    }

    fn on_mempool(&self) {
        trace!("[MEMPOOL] from {}", self.peer.peer_address());

        let state = self.state.read();
        // Query mempool for transactions
        let mut transactions = match &state.remote_subscription {
           Subscription::Addresses(addresses) => self.mempool.get_transactions_by_addresses(addresses.clone(), Self::MEMPOOL_ENTRIES_MAX),
           Subscription::MinFee(min_fee_per_byte) => {
                // NOTE: every integer up to (2^53 - 1) should have an exact representation as f64 (IEEE 754 64-bit double)
                // This is guaranteed by the coin type.
                let min_fee_per_byte: f64 = u64::from(*min_fee_per_byte) as f64;
                self.mempool.get_transactions(Self::MEMPOOL_ENTRIES_MAX, min_fee_per_byte)
            },
           Subscription::Any => {
                self.mempool.get_transactions(Self::MEMPOOL_ENTRIES_MAX, 0f64)
           },
           Subscription::None => return,
        };

        // Send an InvVector for each transaction in the mempool.
        // Split into multiple Inv messages if the mempool is large.
        while !transactions.is_empty() {
            let max_vectors = std::cmp::min(transactions.len(), InvVector::VECTORS_MAX_COUNT);
            let vectors: Vec<InvVector> = transactions.drain(..max_vectors).
                map(|tx| InvVector::from_tx_hash(tx.hash())).
                collect();

            self.peer.channel.send_or_close(Message::Inv(vectors));

            if max_vectors == InvVector::VECTORS_MAX_COUNT {
                std::thread::sleep(Self::MEMPOOL_THROTTLE);
            }
        }
    }

    fn on_not_found(&self, vectors: Vec<InvVector>) {
        trace!("[NOTFOUND] {} vectors", vectors.len());

        // Remove unknown objects from in-flight list.
        let agent = &*self.self_weak;
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
            InvVectorType::Block => state.blocks_to_request.enqueue(vector),
            InvVectorType::Transaction => state.txs_to_request.enqueue(vector),
            InvVectorType::Error => () // XXX Get rid of this!
        }
        self.request_vectors_throttled(&mut *state);
    }

    fn queue_vectors(&self, state: &mut InventoryAgentState, block_vectors: Vec<InvVector>, tx_vectors: Vec<InvVector>) {
        for vector in block_vectors {
            state.blocks_to_request.enqueue(vector);
        }

        for vector in tx_vectors {
            state.txs_to_request.enqueue(vector);
        }

        self.request_vectors_throttled(state);
    }

    fn request_vectors_throttled(&self, state: &mut InventoryAgentState) {
        self.timers.clear_delay(&InventoryAgentTimer::GetDataThrottle);

        if state.blocks_to_request.len() + state.txs_to_request.num_available() > Self::REQUEST_THRESHOLD {
            self.request_vectors(state);
        } else {
            let weak = self.self_weak.clone();
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
        if state.blocks_to_request.is_empty() && !state.txs_to_request.check_available() {
            return;
        }

        // Request queued objects from the peer. Only request up to VECTORS_MAX_COUNT objects at a time.
        let num_blocks = state.blocks_to_request.len().min(Self::REQUEST_VECTORS_MAX);
        let num_txs = Self::REQUEST_VECTORS_MAX - num_blocks; // `dequeue_multi` takes care of the above comparison

        let mut vectors = Vec::new();
        for vector in state.blocks_to_request.dequeue_multi(num_blocks) {
            state.objects_in_flight.insert(vector.clone());
            vectors.push(vector);
        }
        for vector in state.txs_to_request.dequeue_multi(num_txs) {
            state.objects_in_flight.insert(vector.clone());
            vectors.push(vector);
        }

        // Set timeout to detect end of request / missing objects.
        let weak = self.self_weak.clone();
        self.timers.set_delay(InventoryAgentTimer::GetData, move || {
            let this = upgrade_weak!(weak);
            this.no_more_data();
        }, Self::REQUEST_TIMEOUT);

        // Request data from peer.
        self.peer.channel.send_or_close(Message::GetData(vectors));
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
            let weak = self.self_weak.clone();
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
        // Cancel the request timeout timer.
        self.timers.clear_delay(&InventoryAgentTimer::GetData);

        let mut state = self.state.write();

        // TODO optimize
        let inv_mgr_arc = self.inv_mgr.clone();
        let mut inv_mgr = inv_mgr_arc.write();
        let agent = &*self.self_weak;
        let mut vectors = Vec::new();
        for vector in state.objects_in_flight.drain() {
            inv_mgr.note_vector_not_received(agent, &vector);
            vectors.push(vector);
        }

        for vector in vectors {
            state.objects_that_flew.insert(vector);
        }

        // If there are more objects to request, request them.
        if !state.blocks_to_request.is_empty() || state.txs_to_request.check_available() {
            self.request_vectors(&mut *state);
        } else {
            // Give up write lock before notifying.
            drop(state);
            self.sync_protocol.on_all_objects_received();
            self.notifier.read().notify(InventoryEvent::AllObjectsReceived);
        }
    }

    fn on_get_blocks(&self, msg: GetBlocksMessage) {
        {
            let mut state = self.state.write();
            if !state.get_blocks_limit.note_single() {
                warn!("Rejecting GetBlocks message - rate limit exceeded");
                return;
            }
        }

        trace!("[GETBLOCKS] {} block locators max_inv_size {} received from {}", msg.locators.len(), msg.max_inv_size, self.peer.peer_address());

        // A peer has requested blocks. Check all requested block locator hashes
        // in the given order and pick the first hash that is found on our main
        // chain, ignore the rest. If none of the requested hashes is found,
        // pick the genesis block hash. Send the main chain starting from the
        // picked hash back to the peer.
        let network_info = NetworkInfo::from_network_id(self.blockchain.network_id());
        let mut start_block_hash = network_info.genesis_hash().clone();
        for locator in msg.locators.iter() {
            if self.blockchain.get_block(locator, false).is_some() {
                // We found a block, ignore remaining block locator hashes.
                start_block_hash = locator.clone();
                break;
            }
        }

        // Collect up to GETBLOCKS_VECTORS_MAX inventory vectors for the blocks starting right
        // after the identified block on the main chain.
        let blocks = self.blockchain.get_blocks(
            &start_block_hash,
            cmp::min(u32::from(msg.max_inv_size), Self::GET_BLOCKS_VECTORS_MAX),
            false,
            match msg.direction {
                GetBlocksDirection::Forward => Direction::Forward,
                GetBlocksDirection::Backward => Direction::Backward,
            },
        );

        let vectors = blocks.iter().map(|block| {
            InvVector::from_block_hash(block.hash())
        }).collect();

        // Send the vectors back to the requesting peer.
        self.peer.channel.send_or_close(Message::Inv(vectors));
    }

    fn on_get_macro_blocks(&self, msg: GetBlocksMessage) {
        {
            let mut state = self.state.write();
            if !state.get_blocks_limit.note_single() {
                warn!("Rejecting GetBlocks message - rate limit exceeded");
                return;
            }
        }

        trace!("[GETBLOCKS] {} block locators max_inv_size {} received from {}", msg.locators.len(), msg.max_inv_size, self.peer.peer_address());

        // A peer has requested blocks. Check all requested block locator hashes
        // in the given order and pick the first hash that is found on our main
        // chain, ignore the rest. If none of the requested hashes is found,
        // pick the genesis block hash. Send the main chain starting from the
        // picked hash back to the peer.
        let network_info = NetworkInfo::from_network_id(self.blockchain.network_id());
        let mut start_block_hash = network_info.genesis_hash().clone();
        for locator in msg.locators.iter() {
            if self.blockchain.get_block(locator, false).is_some() {
                // We found a block, ignore remaining block locator hashes.
                start_block_hash = locator.clone();
                break;
            }
        }

        // Collect up to GETBLOCKS_VECTORS_MAX inventory vectors for the blocks starting right
        // after the identified block on the main chain.
        let blocks = self.blockchain.get_blocks(
            &start_block_hash,
            cmp::min(u32::from(msg.max_inv_size), Self::GET_BLOCKS_VECTORS_MAX),
            false,
            match msg.direction {
                GetBlocksDirection::Forward => Direction::Forward,
                GetBlocksDirection::Backward => Direction::Backward,
            },
        );

        let vectors = blocks.iter().map(|block| {
            InvVector::from_block_hash(block.hash())
        }).collect();

        // Send the vectors back to the requesting peer.
        self.peer.channel.send_or_close(Message::Inv(vectors));
    }

    fn on_get_data(&self, vectors: Vec<InvVector>) {
        // Keep track of the objects the peer knows.
        {
            let mut state = self.state.write();
            for vector in vectors.iter() {
                state.known_objects.insert(vector.clone());
            }
        }

        // Check which of the requested objects we know.
        // Send back all known objects.
        // Send notFound for unknown objects.
        let mut unknown_objects = Vec::new();

        for vector in vectors {
            match vector.ty {
                InvVectorType::Block => {
                    // TODO raw blocks. Needed?
                    let block_opt = self.blockchain.get_block(&vector.hash, true);
                    match block_opt {
                        Some(block) => {
                            if self.peer.channel.send(P::MessageAdapter::new_block_message(block)).is_err() {
                                self.peer.channel.close(CloseType::SendFailed);
                                return;
                            }
                        },
                        None => {
                            unknown_objects.push(vector);
                        }
                    }
                }
                InvVectorType::Transaction => {
                    let tx_opt = self.mempool.get_transaction(&vector.hash);
                    if tx_opt.is_some() {
                        let tx = Transaction::clone(tx_opt.as_ref().unwrap());
                        if self.peer.channel.send(TxMessage::new(tx)).is_err() {
                            self.peer.channel.close(CloseType::SendFailed);
                            return;
                        }
                    } else {
                        unknown_objects.push(vector);
                    }
                }
                InvVectorType::Error => () // XXX Why do we have this??
            }
        }

        // Report any unknown objects to the sender.
        if !unknown_objects.is_empty() {
            self.peer.channel.send_or_close(Message::NotFound(unknown_objects));
        }
    }

    fn on_get_header(&self, vectors: Vec<InvVector>) {
        // Keep track of the objects the peer knows.
        {
            let mut state = self.state.write();
            for vector in vectors.iter() {
                state.known_objects.insert(vector.clone());
            }
        }

        // Check which of the requested objects we know.
        // Send back all known objects.
        // Send notFound for unknown objects.
        let mut unknown_objects = Vec::new();

        for vector in vectors {
            match vector.ty {
                InvVectorType::Block => {
                    // TODO raw blocks. Needed?
                    let block_opt = self.blockchain.get_block(&vector.hash, false);
                    match block_opt {
                        Some(block) => {
                            if self.peer.channel.send(P::MessageAdapter::new_header_message(block.header())).is_err() {
                                self.peer.channel.close(CloseType::SendFailed);
                                return;
                            }
                        },
                        None => {
                            unknown_objects.push(vector);
                        }
                    }
                }
                InvVectorType::Transaction => {} // XXX JavaScript client errors here
                InvVectorType::Error => {} // XXX Why do we have this??
            }
        }

        // Report any unknown objects to the sender.
        if !unknown_objects.is_empty() {
            self.peer.channel.send_or_close(Message::NotFound(unknown_objects));
        }
    }

    fn on_epoch_transactions(&self, epoch_transactions_message: EpochTransactionsMessage) {
        self.sync_protocol.on_epoch_transactions(epoch_transactions_message);
    }

    pub fn relay_block(&self, block: &<P::Blockchain as AbstractBlockchain<'static>>::Block) -> bool {
        // Only relay block if it matches the peer's subscription.
        if !self.state.read().remote_subscription.matches_block() {
            return false;
        }

        let vector = InvVector::from_block_hash(block.hash());

        // Don't relay block to this peer if it already knows it.
        if self.state.read().known_objects.contains(&vector) {
            return false;
        }

        let mut state = self.state.write();
        // Relay block to peer.
        let mut vectors = state.waiting_tx_inv_vectors.dequeue_multi(InvVector::VECTORS_MAX_COUNT - 1);
        vectors.insert(0, vector.clone());
        self.peer.channel.send_or_close(Message::Inv(vectors));

        // Assume that the peer knows this block now.
        state.known_objects.insert(vector);

        true
    }

    pub fn relay_transaction(&self, transaction: &Transaction) -> bool {
        // Only relay transaction if it matches the peer's subscription.
        if !self.state.read().remote_subscription.matches_transaction(transaction) {
            return false;
        }

        let vector = InvVector::from_tx_hash(transaction.hash());

        // Don't relay transaction to this peer if it already knows it.
        if self.state.read().known_objects.contains(&vector) {
            return false;
        }

        let mut state = self.state.write();
        if (transaction.fee_per_byte() as u64) < Self::TRANSACTION_RELAY_FEE_MIN {
            state.waiting_free_tx_inv_vectors.enqueue(
                FreeTransactionVector::from_vector(&vector, transaction.serialized_size())
            );
        } else {
            state.waiting_tx_inv_vectors.enqueue(vector.clone());
        }

        // Assume that the peer knows this block now.
        state.known_objects.insert(vector);

        true
    }

    pub fn remove_transaction(&self, transaction: &Transaction) {
        let vector = InvVector::from_tx_hash(transaction.hash());
        let mut state = self.state.write();

        // Remove transaction from relay queues.
        state.waiting_tx_inv_vectors.remove(&vector);
        // Serialized size does not matter here due to the implementation of Eq and Hash.
        state.waiting_free_tx_inv_vectors.remove(&FreeTransactionVector::from_vector(&vector, 0));
    }

    fn send_waiting_tx_inv_vectors(&self) {
        let mut state = self.state.write();

        let mut vectors = Vec::new();
        let mut size: usize = 0;
        while vectors.len() <= InvVector::VECTORS_MAX_COUNT && size < Self::FREE_TRANSACTION_SIZE_PER_INTERVAL {
            if let Some(tx) = state.waiting_free_tx_inv_vectors.dequeue() {
                vectors.push(tx.vector);
                size += tx.serialized_size;
            } else {
                break;
            }
        }
        let num_vectors = vectors.len();
        if num_vectors > 0 {
            self.peer.channel.send_or_close(Message::Inv(vectors));
            debug!("Sent {} InvVectors to {}", num_vectors, self.peer.peer_address());
        }
    }

    fn send_waiting_free_tx_inv_vectors(&self) {
        let mut state = self.state.write();

        let mut size: usize = 0;
        let mut vectors = Vec::new();
        while vectors.len() <= InvVector::VECTORS_MAX_COUNT && size < Self::FREE_TRANSACTIONS_PER_SECOND {
            if let Some(vector) = state.waiting_free_tx_inv_vectors.dequeue() {
                size += vector.serialized_size;
                vectors.push(InvVector::from(vector));
            } else {
                break;
            }
        }

        let num_vectors = vectors.len();
        if num_vectors > 0 {
            self.peer.channel.send_or_close(Message::Inv(vectors));
            debug!("Sent {} InvVectors to {}", num_vectors, self.peer.peer_address());
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
