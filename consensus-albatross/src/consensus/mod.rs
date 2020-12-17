// If we don't allow absurd comparisons, clippy fails because `MIN_FULL_NODES` can be 0.
#![allow(clippy::absurd_extreme_comparisons)]

use std::collections::HashMap;
use std::sync::{Arc, Weak};
use std::time::Duration;

use futures::StreamExt;
use parking_lot::{MappedRwLockReadGuard, RwLock, RwLockReadGuard};
use tokio::sync::broadcast::{channel as broadcast, Receiver as BroadcastReceiver, Sender as BroadcastSender};

use block_albatross::Block;
use blockchain_albatross::{Blockchain, BlockchainEvent};
use database::Environment;
use macros::upgrade_weak;
use mempool::{Mempool, MempoolEvent};
use network_interface::{
    network::{Network, NetworkEvent},
    peer::Peer,
};
use transaction::Transaction;
use utils::mutable_once::MutableOnce;

use crate::consensus_agent::ConsensusAgent;
use crate::error::{Error, SyncError};
use crate::sync::SyncProtocol;

mod request_response;

pub enum ConsensusEvent<N: Network> {
    PeerJoined(Arc<ConsensusAgent<N::PeerType>>),
    //PeerLeft(Arc<P>),
    Established,
    Lost,
    Syncing,
    Waiting,
    SyncFailed,
}

impl<N: Network> Clone for ConsensusEvent<N> {
    fn clone(&self) -> Self {
        match self {
            ConsensusEvent::PeerJoined(peer) => ConsensusEvent::PeerJoined(Arc::clone(peer)),
            ConsensusEvent::Established => ConsensusEvent::Established,
            ConsensusEvent::Lost => ConsensusEvent::Lost,
            ConsensusEvent::Syncing => ConsensusEvent::Syncing,
            ConsensusEvent::Waiting => ConsensusEvent::Waiting,
            ConsensusEvent::SyncFailed => ConsensusEvent::SyncFailed,
        }
    }
}

pub struct Consensus<N: Network> {
    pub blockchain: Arc<Blockchain>,
    pub mempool: Arc<Mempool>,
    pub network: Arc<N>,
    pub env: Environment,

    //timers: Timers<ConsensusTimer>,
    pub(crate) state: RwLock<ConsensusState<N>>,

    self_weak: MutableOnce<Weak<Consensus<N>>>,
    events: BroadcastSender<ConsensusEvent<N>>,

    sync_protocol: Box<dyn SyncProtocol<N>>,
}

/*#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum ConsensusTimer {
    Sync,
}*/

type ConsensusAgentMap<P> = HashMap<Arc<P>, Arc<ConsensusAgent<P>>>;

pub(crate) struct ConsensusState<N: Network> {
    pub(crate) agents: ConsensusAgentMap<N::PeerType>,
}

impl<N: Network> Consensus<N> {
    pub const MIN_FULL_NODES: usize = 0;
    const SYNC_THROTTLE: Duration = Duration::from_millis(1500);

    pub fn new<S: SyncProtocol<N>>(
        env: Environment,
        blockchain: Arc<Blockchain>,
        mempool: Arc<Mempool>,
        network: Arc<N>,
        sync_protocol: S,
    ) -> Result<Arc<Self>, Error> {
        let (tx, _rx) = broadcast(256);
        let this = Arc::new(Consensus {
            blockchain,
            mempool,
            network,
            env,

            //timers: Timers::new(),
            state: RwLock::new(ConsensusState { agents: HashMap::new() }),

            self_weak: MutableOnce::new(Weak::new()),
            events: tx,
            sync_protocol: Box::new(sync_protocol),
        });
        Consensus::init_listeners(&this);
        Ok(this)
    }

    pub fn subscribe_events(&self) -> BroadcastReceiver<ConsensusEvent<N>> {
        self.events.subscribe()
    }

    pub fn init_listeners(this: &Arc<Consensus<N>>) {
        unsafe { this.self_weak.replace(Arc::downgrade(this)) };

        Self::init_network_requests(&this.network, &this.blockchain);

        let weak = Arc::downgrade(this);
        let mut stream = this.network.subscribe_events();
        tokio::spawn(async move {
            while let Some(e) = stream.next().await {
                let this = upgrade_weak!(weak);
                match e {
                    Ok(NetworkEvent::PeerJoined(peer)) => this.on_peer_joined(peer),
                    //NetworkEvent::PeerLeft(peer) => this.on_peer_left(Arc::clone(peer)),
                    _ => {}
                }
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
        this.blockchain.register_listener(move |e: &BlockchainEvent| {
            let this = upgrade_weak!(weak);
            this.on_blockchain_event(e);
        });
    }

    fn on_peer_joined(&self, peer: Arc<N::PeerType>) {
        info!("Connected to {:?}", peer.id());
        let agent = Arc::new(ConsensusAgent::new(Arc::clone(&peer)));
        self.state.write().agents.insert(peer, Arc::clone(&agent));

        self.events.send(ConsensusEvent::PeerJoined(agent));
    }

    fn on_peer_left(&self, peer: Arc<N::PeerType>) {
        info!("Disconnected from {:?}", peer.id());
        {
            let mut state = self.state.write();

            state.agents.remove(&peer);
        }

        let weak = Weak::clone(&self.self_weak);
        tokio::spawn(Self::sync_blockchain(weak)); // TODO: Error handling
    }

    fn on_blockchain_event(&self, event: &BlockchainEvent) {
        let state = self.state.read();

        let blocks: Vec<&Block>;
        let block;
        match event {
            BlockchainEvent::Extended(_) | BlockchainEvent::Finalized(_) => {
                // This implicitly takes the lock on the blockchain state.
                block = self.blockchain.head();
                blocks = vec![&block];
            }
            BlockchainEvent::Rebranched(_, ref adopted_blocks) => {
                blocks = adopted_blocks.iter().map(|(_, block)| block).collect();
            }
            _ => return,
        }

        // print block height
        let height = self.blockchain.block_number();
        if height % 100 == 0 {
            info!("Now at block #{}", height);
        } else {
            trace!("Now at block #{}", height);
        }

        // Only relay blocks if we are synced up.
        if self.sync_protocol.is_established() {
            for agent in state.agents.values() {
                for &block in blocks.iter() {
                    agent.relay_block(block);
                }
            }
        }
    }

    fn on_transaction_added(&self, transaction: &Arc<Transaction>) {
        let state = self.state.read();

        // Don't relay transactions if we are not synced yet.
        if !self.sync_protocol.is_established() {
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

    pub async fn sync_blockchain(this: Weak<Self>) -> Result<(), SyncError> {
        info!("Syncing blockchain!");
        if let Some(this) = Weak::upgrade(&this) {
            let this2 = Arc::clone(&this);
            this.sync_protocol.perform_sync(this2).await
        } else {
            Ok(())
        }
    }

    pub fn established(&self) -> bool {
        self.sync_protocol.is_established()
    }

    pub fn num_agents(&self) -> usize {
        self.state.read().agents.len()
    }

    pub fn agents(&self) -> MappedRwLockReadGuard<ConsensusAgentMap<N::PeerType>> {
        let state = self.state.read();
        RwLockReadGuard::map(state, |state| &state.agents)
    }
}
