use std::collections::HashMap;
use std::sync::{Arc, Weak};
use std::time::Duration;

use parking_lot::{Mutex, RwLock};
use rand::{Rng, rngs::OsRng};

use blockchain::Blockchain;
use database::Environment;
use mempool::Mempool;
use network::{Network, NetworkConfig, NetworkEvent, Peer};
use network_primitives::networks::NetworkId;
use network_primitives::time::NetworkTime;
use utils::mutable_once::MutableOnce;
use utils::observer::Notifier;
use utils::timers::Timers;

use crate::consensus_agent::ConsensusAgent;
use crate::consensus_agent::ConsensusAgentEvent;
use crate::inventory::InventoryManager;

pub struct Consensus {
    pub blockchain: Arc<Blockchain<'static>>,
    pub mempool: Arc<Mempool<'static>>,
    pub network: Arc<Network>,

    inv_mgr: Arc<RwLock<InventoryManager>>,
    timers: Timers<ConsensusTimer>,

    state: RwLock<ConsensusState>,

    self_weak: MutableOnce<Weak<Consensus>>,
    pub notifier: RwLock<Notifier<'static, ConsensusEvent>>,

    sync_lock: Mutex<()>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum ConsensusEvent {
    Established,
    Lost,
    Syncing,
    Waiting,
    SyncFailed,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum ConsensusTimer {
    Sync,
}

struct ConsensusState {
    established: bool,
    agents: HashMap<Arc<Peer>, Arc<RwLock<ConsensusAgent>>>,

    sync_peer: Option<Arc<Peer>>,
}

impl Consensus {
    const MIN_FULL_NODES: usize = 1;
    const SYNC_THROTTLE: Duration = Duration::from_millis(1500);

    pub fn new(env: &'static Environment, network_id: NetworkId, network_config: NetworkConfig) -> Arc<Self> {
        let network_time = Arc::new(NetworkTime::new());
        let blockchain = Arc::new(Blockchain::new(env, network_id, network_time.clone()));
        let mempool = Mempool::new(blockchain.clone());
        let network = Network::new(blockchain.clone(), network_config, network_time, network_id);

        let this = Arc::new(Consensus {
            blockchain,
            mempool,
            network,

            inv_mgr: InventoryManager::new(),
            timers: Timers::new(),

            state: RwLock::new(ConsensusState {
                established: false,
                agents: HashMap::new(),

                sync_peer: None,
            }),

            self_weak: MutableOnce::new(Weak::new()),
            notifier: RwLock::new(Notifier::new()),

            sync_lock: Mutex::new(()),
        });
        Consensus::init_listeners(&this);
        this
    }

    fn init_listeners(this: &Arc<Consensus>) {
        unsafe { this.self_weak.replace(Arc::downgrade(this)) };

        let weak = Arc::downgrade(this);
        this.network.notifier.write().register(move |e: NetworkEvent| {
            let this = upgrade_weak!(weak);
            match e {
                NetworkEvent::PeerJoined(peer) => this.on_peer_joined(peer),
                NetworkEvent::PeerLeft(peer) => this.on_peer_left(peer),
                _ => {}
            }
        });
    }

    fn on_peer_joined(&self, peer: Peer) {
        let peer_arc = Arc::new(peer);
        let agent = ConsensusAgent::new(
            self.blockchain.clone(),
            self.mempool.clone(),
            self.inv_mgr.clone(),
            peer_arc.clone());

        let weak = self.self_weak.clone();
        let peer_arc_moved = peer_arc.clone();
        agent.read().notifier.write().register(move |e: &ConsensusAgentEvent| {
            let this = upgrade_weak!(weak);
            match e {
                ConsensusAgentEvent::Synced => this.on_peer_synced(peer_arc_moved.clone()),
            }
        });

        // If no more peers connect within the specified timeout, start syncing.
        let weak = self.self_weak.clone();
        self.timers.reset_delay(ConsensusTimer::Sync, move || {
            let this = upgrade_weak!(weak);
            this.sync_blockchain();
        }, Self::SYNC_THROTTLE);

        self.state.write().agents.insert(peer_arc, agent);
    }

    fn on_peer_left(&self, peer: Peer) {
        let sync_guard = self.sync_lock.lock();
        let peer = Arc::new(peer);

        // Reset syncPeer if it left during the sync.
        if self.state.read().sync_peer.as_ref().map_or(false, |sync_peer| sync_peer == &peer) {
            debug!("Peer {} left during sync", peer.peer_address());
            self.state.write().sync_peer = None;
            self.notifier.read().notify(ConsensusEvent::SyncFailed);
        }

        self.state.write().agents.remove(&peer);
        self.sync_blockchain();
    }

    fn on_peer_synced(&self, peer: Arc<Peer>) {
        // Reset syncPeer if we finished syncing with it.
        if self.state.read().sync_peer.as_ref().map_or(false, |sync_peer| sync_peer == &peer) {
            trace!("Finished sync with peer {}", peer.peer_address());
            self.state.write().sync_peer = None;
        }

        self.sync_blockchain();
    }

    fn sync_blockchain(&self) {
        let sync_guard = self.sync_lock.lock();

        let mut num_synced_full_nodes: usize = 0;
        let agent: Option<Arc<RwLock<ConsensusAgent>>>;
        let mut consensus_lost: bool;
        let established: bool;
        let num_agents: usize;
        {
            let mut state = self.state.read();

            let candidates: Vec<&Arc<RwLock<ConsensusAgent>>> = state.agents.values()
                .filter(|&agent| {
                    let agent = agent.read();
                    let synced = agent.synced();
                    if synced && agent.peer.peer_address().services.is_full_node() {
                        num_synced_full_nodes += 1;
                    }
                    !synced
                }).collect();

            // Choose a random peer which we aren't sync'd with yet.
            let mut cspring: OsRng = OsRng::new().unwrap();
            agent = cspring.choose(&candidates).map(|&agent| agent.clone());

            // Report consensus-lost if we are synced with less than the minimum number of full nodes or have no connections at all.
            consensus_lost = state.established && (num_synced_full_nodes < Self::MIN_FULL_NODES || state.agents.is_empty());

            // Wait for ongoing sync to finish.
            if state.sync_peer.is_some() {
                return;
            }

            established = state.established;
            num_agents = state.agents.len();
        }

        if consensus_lost {
            self.state.write().established = false;
            self.notifier.read().notify(ConsensusEvent::Lost);
        }

        if let Some(agent) = agent {
            let agent = agent.read();
            self.state.write().sync_peer = Some(agent.peer.clone());

            // Notify listeners when we start syncing and have not established consensus yet.
            if !established {
                self.notifier.read().notify(ConsensusEvent::Syncing);
            }

            trace!("Syncing blockchain with peer {}", agent.peer.peer_address());
            drop(sync_guard);
            agent.sync();
        } else {
            // We are synced with all connected peers.

            // Report consensus-established if we are connected to the minimum number of full nodes.
            if num_synced_full_nodes >= Self::MIN_FULL_NODES {
                if !established {
                    trace!("Synced with all connected peers ({}), consensus established.", num_agents);
                    debug!("Blockchain: height={}, head_hash={:?}", self.blockchain.height(), self.blockchain.head_hash());

                    // Report consensus-established.
                    self.state.write().established = true;
                    self.notifier.read().notify(ConsensusEvent::Established);

                    // Allow inbound network connections after establishing consensus.
                    self.network.set_allow_inbound_connections(true);
                }
            } else {
                trace!("Waiting for more peer connections to be established.");
                // Otherwise, wait until more peer connections are established.
                self.notifier.read().notify(ConsensusEvent::Waiting);
            }
        }
    }
}