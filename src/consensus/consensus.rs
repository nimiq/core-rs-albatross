use std::sync::{Arc, Weak};
use parking_lot::RwLock;

use crate::consensus::base::blockchain::Blockchain;
use crate::consensus::base::mempool::Mempool;
use crate::consensus::consensus_agent::ConsensusAgent;
use crate::consensus::inventory::InventoryManager;
use crate::consensus::networks::NetworkId;
use crate::network::{Network, NetworkConfig, NetworkTime, NetworkEvent, Peer};
use crate::utils::db::Environment;
use crate::utils::observer::Notifier;
use crate::utils::timers::Timers;

pub struct Consensus {
    pub blockchain: Arc<Blockchain<'static>>,
    pub mempool: Arc<Mempool<'static>>,
    pub network: Arc<Network>,

    inv_mgr: Arc<RwLock<InventoryManager>>,
    timers: Timers<ConsensusTimer>,

    state: RwLock<ConsensusState>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum ConsensusEvent {
    Established,
    Lost,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum ConsensusTimer {

}

struct ConsensusState {
    established: bool,
    agents: Vec<Arc<RwLock<ConsensusAgent>>>,

    notifier: Notifier<'static, ConsensusEvent>,
    self_weak: Weak<Consensus>,
}

impl Consensus {
    pub fn new(env: &'static Environment, network_id: NetworkId, network_config: NetworkConfig) -> Arc<Self> {
        let network_time = Arc::new(NetworkTime::new());
        let blockchain = Arc::new(Blockchain::new(env, network_id, network_time.clone()));
        let mempool = Mempool::new(blockchain.clone());
        let network = Network::new(blockchain.clone(), network_config, network_time);

        let this = Arc::new(Consensus {
            blockchain,
            mempool,
            network,

            inv_mgr: InventoryManager::new(),
            timers: Timers::new(),

            state: RwLock::new(ConsensusState {
                established: false,
                agents: Vec::new(),

                notifier: Notifier::new(),
                self_weak: Weak::new(),
            })
        });
        Consensus::init_listeners(&this);
        this
    }

    fn init_listeners(this: &Arc<Consensus>) {
        this.state.write().self_weak = Arc::downgrade(this);

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
        let agent = ConsensusAgent::new(
            self.blockchain.clone(),
            self.mempool.clone(),
            self.inv_mgr.clone(),
            peer);

        agent.write().sync();

        self.state.write().agents.push(agent);
    }

    fn on_peer_left(&self, peer: Peer) {

    }
}
