use std::collections::HashMap;
use std::sync::{Arc, Weak};
use std::time::Duration;

use parking_lot::RwLock;
use rand::thread_rng;
use rand::seq::SliceRandom;

use blockchain::{Blockchain, BlockchainEvent};
use database::Environment;
use mempool::{Mempool, MempoolEvent};
use network::{Network, NetworkConfig, NetworkEvent, Peer};
use network_primitives::networks::NetworkId;
use network_primitives::time::NetworkTime;
use transaction::Transaction;
use utils::mutable_once::MutableOnce;
use utils::observer::Notifier;
use utils::timers::Timers;

use crate::consensus_agent::ConsensusAgent;
use crate::consensus_agent::ConsensusAgentEvent;
use crate::inventory::InventoryManager;
use crate::error::Error;
use crate::accounts_chunk_cache::AccountsChunkCache;


pub struct Consensus {
    pub blockchain: Arc<Blockchain<'static>>,
    pub mempool: Arc<Mempool<'static>>,
    pub network: Arc<Network>,

    inv_mgr: Arc<RwLock<InventoryManager>>,
    timers: Timers<ConsensusTimer>,
    accounts_chunk_cache: Arc<AccountsChunkCache>,

    state: RwLock<ConsensusState>,

    self_weak: MutableOnce<Weak<Consensus>>,
    pub notifier: RwLock<Notifier<'static, ConsensusEvent>>,
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
    agents: HashMap<Arc<Peer>, Arc<ConsensusAgent>>,

    sync_peer: Option<Arc<Peer>>,
}

impl Consensus {
    const MIN_FULL_NODES: usize = 1;
    const SYNC_THROTTLE: Duration = Duration::from_millis(1500);

    pub fn new(env: &'static Environment, network_id: NetworkId, network_config: NetworkConfig) -> Result<Arc<Self>, Error> {
        let network_time = Arc::new(NetworkTime::new());
        let blockchain = Arc::new(Blockchain::new(env, network_id, network_time.clone()));
        let mempool = Mempool::new(blockchain.clone());
        let network = Network::new(blockchain.clone(), network_config, network_time, network_id)?;
        let accounts_chunk_cache = AccountsChunkCache::new(env, Arc::clone(&blockchain));

        let this = Arc::new(Consensus {
            blockchain,
            mempool,
            network,

            inv_mgr: InventoryManager::new(),
            timers: Timers::new(),
            accounts_chunk_cache,

            state: RwLock::new(ConsensusState {
                established: false,
                agents: HashMap::new(),

                sync_peer: None,
            }),

            self_weak: MutableOnce::new(Weak::new()),
            notifier: RwLock::new(Notifier::new()),
        });
        Consensus::init_listeners(&this);
        Ok(this)
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

        // Relay new (verified) transactions to peers.
        let weak = Arc::downgrade(this);
        this.mempool.notifier.write().register(move |e: &MempoolEvent| {
            let this = upgrade_weak!(weak);
            match e {
                MempoolEvent::TransactionAdded(_, transaction) => this.on_transaction_added(transaction),
                // TODO: Relay on restore?
                MempoolEvent::TransactionRestored(transaction) => this.on_transaction_added(transaction),
                MempoolEvent::TransactionEvicted(transaction) => this.on_transaction_removed(transaction),
                MempoolEvent::TransactionMined(transaction) => this.on_transaction_removed(transaction),
            }
        });

        // Notify peers when our blockchain head changes.
        let weak = Arc::downgrade(this);
        this.blockchain.notifier.write().register(move |e: &BlockchainEvent| {
            let this = upgrade_weak!(weak);
            this.on_blockchain_event(e);
        });
    }

    fn on_peer_joined(&self, peer: Peer) {
        let peer_arc = Arc::new(peer);
        let agent = ConsensusAgent::new(
            self.blockchain.clone(),
            self.mempool.clone(),
            self.inv_mgr.clone(),
            self.accounts_chunk_cache.clone(),
            peer_arc.clone());

        let weak = self.self_weak.clone();
        let peer_arc_moved = peer_arc.clone();
        agent.notifier.write().register(move |e: &ConsensusAgentEvent| {
            let this = upgrade_weak!(weak);
            match e {
                ConsensusAgentEvent::Synced => this.on_peer_synced(peer_arc_moved.clone()),
                ConsensusAgentEvent::OutOfSync => this.on_peer_out_of_sync(peer_arc_moved.clone()),
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
        {
            let mut state = self.state.write();

            let peer = Arc::new(peer);
            state.agents.remove(&peer);

            // Reset syncPeer if it left during the sync.
            if state.sync_peer.as_ref().map_or(false, |sync_peer| sync_peer == &peer) {
                debug!("Peer {} left during sync", peer.peer_address());
                state.sync_peer = None;
                drop(state);

                self.notifier.read().notify(ConsensusEvent::SyncFailed);
            }
        }

        self.sync_blockchain();
    }

    fn on_peer_synced(&self, peer: Arc<Peer>) {
        // Reset syncPeer if we finished syncing with it.
        {
            let mut state = self.state.write();
            if state.sync_peer.as_ref().map_or(false, |sync_peer| sync_peer == &peer) {
                trace!("Finished sync with peer {}", peer.peer_address());
                state.sync_peer = None;
            }
        }

        self.sync_blockchain();
    }

    fn on_peer_out_of_sync(&self, peer: Arc<Peer>) {
        warn!("Peer {} out of sync, resyncing", peer.peer_address());
        self.sync_blockchain();
    }

    fn on_blockchain_event(&self, event: &BlockchainEvent) {
        let state = self.state.read();

        // Don't relay transactions if we are not synced yet.
        if !state.established {
            return;
        }

        let blocks;
        match event {
            BlockchainEvent::Extended(_, ref block) => {
                blocks = vec![block.as_ref()];
            },
            BlockchainEvent::Rebranched(_, ref adopted_blocks) => {
                blocks = adopted_blocks.iter().map(|(_, block)| block).collect();
            }
        }

        for agent in state.agents.values() {
            for &block in blocks.iter() {
                agent.relay_block(block);
            }
        }
    }

    fn on_transaction_added(&self, transaction: &Arc<Transaction>) {
        let state = self.state.read();

        // Don't relay transactions if we are not synced yet.
        if !state.established {
            return;
        }

        for agent in state.agents.values() {
            agent.relay_transaction(transaction.as_ref());
        }
    }

    fn on_transaction_removed(&self, transaction: &Arc<Transaction>) {
        let state = self.state.read();
        for agent in state.agents.values() {
            agent.remove_transaction(transaction.as_ref());
        }
    }

    fn sync_blockchain(&self) {
        let mut state = self.state.write();

        let mut num_synced_full_nodes: usize = 0;
        let candidates: Vec<&Arc<ConsensusAgent>> = state.agents.values()
            .filter(|&agent| {
                let synced = agent.synced();
                if synced && agent.peer.peer_address().services.is_full_node() {
                    num_synced_full_nodes += 1;
                }
                !synced
            }).collect();

        // Choose a random peer which we aren't sync'd with yet.
        let mut rng = thread_rng();
        let agent = candidates.choose(&mut rng).map(|&agent| agent.clone());

        // Report consensus-lost if we are synced with less than the minimum number of full nodes or have no connections at all.
        let consensus_lost = state.established && (num_synced_full_nodes < Self::MIN_FULL_NODES || state.agents.is_empty());

        // Wait for ongoing sync to finish.
        if state.sync_peer.is_some() {
            return;
        }

        let established = state.established;
        let num_agents = state.agents.len();

        if consensus_lost {
            state.established = false;
            // FIXME we're still holding state write lock when notifying here.
            self.notifier.read().notify(ConsensusEvent::Lost);
        }

        if let Some(agent) = agent {
            state.sync_peer = Some(agent.peer.clone());
            drop(state);

            // Notify listeners when we start syncing and have not established consensus yet.
            if !established {
                self.notifier.read().notify(ConsensusEvent::Syncing);
            }

            trace!("Syncing blockchain with peer {}", agent.peer.peer_address());
            agent.sync();
        } else {
            // We are synced with all connected peers.

            // Report consensus-established if we are connected to the minimum number of full nodes.
            if num_synced_full_nodes >= Self::MIN_FULL_NODES {
                if !established {
                    trace!("Synced with all connected peers ({}), consensus established.", num_agents);
                    debug!("Blockchain: height={}, head_hash={:?}", self.blockchain.height(), self.blockchain.head_hash());

                    state.established = true;
                    drop(state);

                    // Report consensus-established.
                    self.notifier.read().notify(ConsensusEvent::Established);

                    // Allow inbound network connections after establishing consensus.
                    self.network.set_allow_inbound_connections(true);
                }
            } else {
                trace!("Waiting for more peer connections to be established.");
                drop(state);

                // Otherwise, wait until more peer connections are established.
                self.notifier.read().notify(ConsensusEvent::Waiting);
            }
        }
    }
}
