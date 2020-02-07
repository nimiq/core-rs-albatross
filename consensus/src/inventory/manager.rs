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
use macros::upgrade_weak;
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
use crate::inventory::InventoryAgent;


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

    pub(crate) fn ask_to_request_vector(&mut self, agent: &InventoryAgent<P>, vector: &InvVector) {
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

            self.timers.clear_delay(&InventoryManagerTimer::Request(vector.clone()));
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

    pub(crate) fn note_vector_received(&mut self, vector: &InvVector) {
        self.timers.clear_delay(&InventoryManagerTimer::Request(vector.clone()));
        self.vectors_to_request.remove(vector);
    }

    pub(crate) fn note_vector_not_received(&mut self, agent_weak: &Weak<InventoryAgent<P>>, vector: &InvVector) {
        self.timers.clear_delay(&InventoryManagerTimer::Request(vector.clone()));

        let record_opt = self.vectors_to_request.get_mut(vector);
        let caller_opt = agent_weak.upgrade();
        if record_opt.is_none() || caller_opt.is_none() {
            return;
        }

        let record = record_opt.unwrap();
        let current_opt = record.0.upgrade();
        let caller = caller_opt.unwrap();

        record.1.remove(&caller);

        if let Some(ref current) = current_opt {
            if !current.peer.channel.closed() && !Arc::ptr_eq(&caller, current) {
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

        debug!("Active agent {:?} didn't find {:?} {}, trying {:p} ({} agents left)",
               current_opt.map_or_else(|| "null".to_string(), |c| format!("{:p}", c)),
               vector.ty, vector.hash, next_agent, record.1.len());

        self.request_vector(&next_agent, vector);
    }
}
