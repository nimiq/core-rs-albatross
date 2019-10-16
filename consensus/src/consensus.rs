// If we don't allow absurd comparisons, clippy fails because `MIN_FULL_NODES` can be 0.
#![allow(clippy::absurd_extreme_comparisons)]

use std::collections::HashMap;
use std::sync::{Arc, Weak};
use std::time::Duration;

use parking_lot::RwLock;
use rand::seq::SliceRandom;
use rand::thread_rng;

use blockchain_base::{AbstractBlockchain, BlockchainEvent};
use database::Environment;
use mempool::{Mempool, MempoolEvent, MempoolConfig};
use network::{Network, NetworkConfig, NetworkEvent, Peer};
use network_primitives::networks::NetworkId;
use network_primitives::time::NetworkTime;
use transaction::Transaction;
use utils::mutable_once::MutableOnce;
use utils::observer::Notifier;
use utils::timers::Timers;

use crate::accounts_chunk_cache::AccountsChunkCache;
use crate::consensus_agent::{ConsensusAgent, ConsensusAgentEvent};
use crate::error::Error;
use crate::inventory::InventoryManager;
use crate::protocol::ConsensusProtocol;

pub struct Consensus<P: ConsensusProtocol + 'static> {
    pub blockchain: Arc<P::Blockchain>,
    pub mempool: Arc<Mempool<'static, P::Blockchain>>,
    pub network: Arc<Network<P::Blockchain>>,
    pub env: &'static Environment,

    inv_mgr: Arc<RwLock<InventoryManager<P>>>,
    timers: Timers<ConsensusTimer>,
    accounts_chunk_cache: Arc<AccountsChunkCache<P::Blockchain>>,

    state: RwLock<ConsensusState<P>>,

    self_weak: MutableOnce<Weak<Consensus<P>>>,
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

type ConsensusAgentMap<P> = HashMap<Arc<Peer>, Arc<ConsensusAgent<P>>>;

struct ConsensusState<P: ConsensusProtocol + 'static> {
    established: bool,
    agents: ConsensusAgentMap<P>,

    sync_peer: Option<Arc<Peer>>,
}

impl<P: ConsensusProtocol + 'static> Consensus<P> {
    const MIN_FULL_NODES: usize = 0;
    const SYNC_THROTTLE: Duration = Duration::from_millis(1500);

    pub fn new(env: &'static Environment, network_id: NetworkId, network_config: NetworkConfig, mempool_config: MempoolConfig) -> Result<Arc<Self>, Error> {
        let network_time = Arc::new(NetworkTime::new());
        let blockchain = Arc::new(<P::Blockchain as AbstractBlockchain<'static>>::new(env, network_id, Arc::clone(&network_time))?);
        let mempool = Mempool::new(blockchain.clone(), mempool_config);
        let network = Network::new(blockchain.clone(), network_config, network_time, network_id)?;
        let accounts_chunk_cache = AccountsChunkCache::new(env, Arc::clone(&blockchain));

        let this = Arc::new(Consensus {
            blockchain,
            mempool,
            network,
            env,

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

    pub fn init_listeners(this: &Arc<Consensus<P>>) {
        unsafe { this.self_weak.replace(Arc::downgrade(this)) };

        let weak = Arc::downgrade(this);
        this.network.notifier.write().register(move |e: &NetworkEvent| {
            let this = upgrade_weak!(weak);
            match e {
                NetworkEvent::PeerJoined(peer) => this.on_peer_joined(Arc::clone(peer)),
                NetworkEvent::PeerLeft(peer) => this.on_peer_left(Arc::clone(peer)),
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
        this.blockchain.register_listener(move |e: &BlockchainEvent<<P::Blockchain as AbstractBlockchain<'static>>::Block>| {
            let this = upgrade_weak!(weak);
            this.on_blockchain_event(e);
        });
    }

    fn on_peer_joined(&self, peer: Arc<Peer>) {
        info!("Connected to {}", peer.peer_address());
        let agent = ConsensusAgent::new(
            self.blockchain.clone(),
            self.mempool.clone(),
            self.inv_mgr.clone(),
            self.accounts_chunk_cache.clone(),
            peer.clone());

        let weak = self.self_weak.clone();
        let peer_arc_moved = peer.clone();
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

        self.state.write().agents.insert(peer, agent);
    }

    fn on_peer_left(&self, peer: Arc<Peer>) {
        info!("Disconnected from {}", peer.peer_address());
        {
            let mut state = self.state.write();

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
                debug!("Finished sync with peer {}", peer.peer_address());
                state.sync_peer = None;
            }
        }

        self.sync_blockchain();
    }

    fn on_peer_out_of_sync(&self, peer: Arc<Peer>) {
        warn!("Peer {} out of sync, re-syncing", peer.peer_address());
        self.sync_blockchain();
    }

    fn on_blockchain_event(&self, event: &BlockchainEvent<<P::Blockchain as AbstractBlockchain<'static>>::Block>) {
        let state = self.state.read();

        let blocks: Vec<&<P::Blockchain as AbstractBlockchain<'static>>::Block>;
        let block;
        match event {
            BlockchainEvent::Extended(_) | BlockchainEvent::Finalized(_) => {
                // This implicitly takes the lock on the blockchain state.
                block = self.blockchain.head_block();
                blocks = vec![&block];
            },
            BlockchainEvent::Rebranched(_, ref adopted_blocks) => {
                blocks = adopted_blocks.iter().map(|(_, block)| block).collect();
            },
        }

        // Don't relay blocks if we are not synced yet.
        if !state.established {
            let height = self.blockchain.head_height();
            if height % 100 == 0 {
                info!("Now at block #{}", height);
            }
            return;
        } else {
            info!("Now at block #{}", self.blockchain.head_height());
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

        // Wait for ongoing sync to finish.
        if state.sync_peer.is_some() {
            return;
        }

        let mut num_synced_full_nodes: usize = 0;
        let candidates: Vec<&Arc<ConsensusAgent<P>>> = state.agents.values()
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

        // Report consensus-lost if we are synced with less than the minimum number of full nodes.
        if state.established && num_synced_full_nodes < Self::MIN_FULL_NODES {
            state.established = false;
            info!("Consensus lost");
            // FIXME we're still holding state write lock when notifying here.
            self.notifier.read().notify(ConsensusEvent::Lost);
        }

        if let Some(agent) = agent {
            state.sync_peer = Some(agent.peer.clone());
            let established = state.established;
            drop(state);

            // Notify listeners when we start syncing and have not established consensus yet.
            if !established {
                self.notifier.read().notify(ConsensusEvent::Syncing);
            }

            debug!("Syncing blockchain with peer {}", agent.peer.peer_address());
            agent.sync();
        } else {
            // We are synced with all connected peers.
            // Report consensus-established if we are connected to the minimum number of full nodes.
            if num_synced_full_nodes >= Self::MIN_FULL_NODES {
                if !state.established {
                    info!("Synced with all connected peers ({}), consensus established", state.agents.len());
                    info!("Blockchain at block #{} [{}]", self.blockchain.head_height(), self.blockchain.head_hash());

                    state.established = true;
                    drop(state);

                    // Report consensus-established.
                    self.notifier.read().notify(ConsensusEvent::Established);

                    // Allow inbound network connections after establishing consensus.
                    self.network.set_allow_inbound_connections(true);
                }
            } else {
                info!("Waiting for more peer connections...");
                drop(state);

                // Otherwise, wait until more peer connections are established.
                self.notifier.read().notify(ConsensusEvent::Waiting);
            }
        }
    }

    pub fn established(&self) -> bool {
        self.state.read().established
    }
}
