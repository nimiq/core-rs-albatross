use parking_lot::RwLock;
use std::sync::{Arc, Weak};
use std::time::{Instant, Duration};

use crate::consensus::base::blockchain::{Blockchain, PushResult};
use crate::consensus::base::mempool::Mempool;
use crate::consensus::base::Subscription;
use hash::Blake2bHash;
use crate::consensus::inventory::{InventoryManager, InventoryAgent, InventoryEvent};
use crate::network::Peer;
use crate::network::connection::close_type::CloseType;
use crate::utils::observer::Notifier;
use crate::utils::mutable_once::MutableOnce;
use crate::utils::timers::Timers;
use parking_lot::Mutex;
use parking_lot::MutexGuard;
use rand::Rng;

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ConsensusAgentEvent {
    Synced
}

pub struct ConsensusAgentState {
    /// Flag indicating that we are currently syncing our blockchain with the peer's.
    syncing: bool,

    /// Flag indicating that we have synced our blockchain with the peer's.
    pub synced: bool,

    /// The hash of the block that we want to learn to consider the sync complete.
    sync_target: Blake2bHash,

    /// The hash of the last fork block the peer has sent us.
    fork_head: Option<Blake2bHash>,

    /// The number of blocks that extended our blockchain since the last requestBlocks().
    num_blocks_extending: u32,

    /// The number of blocks that forked our blockchain since the last requestBlocks().
    num_blocks_forking: u32,

    /// The number of failed blockchain sync attempts.
    failed_syncs: u32,
}

#[derive(Ord, PartialOrd, PartialEq, Eq, Hash, Clone, Copy, Debug)]
enum ConsensusAgentTimer {
    Mempool,
}


pub struct ConsensusAgent {
    blockchain: Arc<Blockchain<'static>>,
    mempool: Arc<Mempool<'static>>,
    pub peer: Arc<Peer>,

    inv_agent: Arc<InventoryAgent>,

    state: RwLock<ConsensusAgentState>,

    pub notifier: RwLock<Notifier<'static, ConsensusAgentEvent>>,
    self_weak: Weak<RwLock<ConsensusAgent>>,

    sync_lock: Mutex<()>,

    timers: Timers<ConsensusAgentTimer>
}

impl ConsensusAgent {
    const SYNC_ATTEMPTS_MAX: u32 = 25;
    const GET_BLOCKS_TIMEOUT: Duration = Duration::from_secs(10);
    const GET_BLOCKS_MAX_RESULTS: u16 = 500;

    /// Minimum time to wait before triggering the initial mempool request.
    const MEMPOOL_DELAY_MIN: u64 = 2 * 1000; // in ms
    /// Maximum time to wait before triggering the initial mempool request.
    const MEMPOOL_DELAY_MAX: u64 = 20 * 1000; // in ms

    pub fn new(blockchain: Arc<Blockchain<'static>>, mempool: Arc<Mempool<'static>>, inv_mgr: Arc<RwLock<InventoryManager>>, peer: Arc<Peer>) -> Arc<RwLock<Self>> {
        let sync_target = peer.head_hash.clone();
        let peer_arc = peer;
        let inv_agent = InventoryAgent::new(blockchain.clone(), mempool.clone(), inv_mgr,peer_arc.clone());
        let this = Arc::new(RwLock::new(ConsensusAgent {
            blockchain,
            mempool,
            peer: peer_arc.clone(),
            inv_agent,

            state: RwLock::new(ConsensusAgentState {
                syncing: false,
                synced: false,
                sync_target,
                fork_head: None,
                // Initialize to 1 to not count the initial sync call as a failed attempt.
                num_blocks_extending: 1,
                num_blocks_forking: 0,
                failed_syncs: 0,
            }),

            // TODO whole agent is locked, thus we can remove this lock
            notifier: RwLock::new(Notifier::new()),
            self_weak: Weak::new(),

            sync_lock: Mutex::new(()),
            timers: Timers::new()
        }));
        ConsensusAgent::init_listeners(&this);
        this
    }

    pub fn synced(&self) -> bool {
        self.state.read().synced
    }

    fn init_listeners(this: &Arc<RwLock<Self>>) {
        this.write().self_weak = Arc::downgrade(this);

        let weak = Arc::downgrade(this);
        this.read().inv_agent.notifier.write().register(move |e: &InventoryEvent| {
            let this = upgrade_weak!(weak);
            this.read().on_inventory_event(e);
        });
    }

    pub fn sync(&self) {
        self.state.write().syncing = true;

        // Don't go through the InventoryManager when syncing.
        self.inv_agent.bypass_mgr(true);

        self.perform_sync();
    }

    fn perform_sync(&self) {
        let sync_guard = self.sync_lock.lock();

        // Wait for ongoing requests to finish.
        if self.inv_agent.is_busy() {
            return;
        }

        // If we know our sync target block, the sync is finished.
        if self.blockchain.contains(&self.state.read().sync_target, true) {
            self.sync_finished(sync_guard);
            return;
        }

        // If the peer didn't send us any blocks that extended our chain, count it as a failed sync attempt.
        // This sets a maximum length for forks that the full client will accept:
        //   FullConsensusAgent.SYNC_ATTEMPTS_MAX * BaseInvectoryMessage.VECTORS_MAX_COUNT
        {
            let mut state = self.state.write();
            if state.num_blocks_extending == 0 {
                state.failed_syncs += 1;
                if state.failed_syncs >= ConsensusAgent::SYNC_ATTEMPTS_MAX {
                    self.peer.channel.close(CloseType::BlockchainSyncFailed);
                    return;
                }
            }
        }

        // We don't know the peer's head block, request blocks from it.
        self.request_blocks();
    }

    fn sync_finished(&self, sync_guard: MutexGuard<()>) {
        // Subscribe to all announcements from the peer.
        self.inv_agent.subscribe(Subscription::Any);

        // Request the peer's mempool.
        // XXX Use a random delay here to prevent requests to multiple peers at once.
        let weak = self.self_weak.clone();
        self.timers.set_delay(ConsensusAgentTimer::Mempool, move || {
            let arc = upgrade_weak!(weak);
            let agent = arc.read();
            agent.timers.clear_delay(&ConsensusAgentTimer::Mempool);
            agent.inv_agent.mempool();
        }, Duration::from_millis(rand::thread_rng()
            .gen_range(ConsensusAgent::MEMPOOL_DELAY_MIN, ConsensusAgent::MEMPOOL_DELAY_MAX)));


        self.inv_agent.bypass_mgr(false);

        {
            let mut state = self.state.write();
            state.syncing = false;
            state.synced = true;

            state.num_blocks_extending = 1;
            state.num_blocks_forking = 0;
            state.fork_head = None;
            state.failed_syncs = 0;
        }

        drop(sync_guard);
        self.notifier.read().notify(ConsensusAgentEvent::Synced);
    }

    fn request_blocks(&self) {
        let locators;
        {
            let state = self.state.read();
            // Check if the peer is sending us a fork.
            let on_fork = state.fork_head.is_some() && state.num_blocks_extending == 0 && state.num_blocks_forking > 0;

            locators = match on_fork {
                true => vec![state.fork_head.as_ref().unwrap().clone()],
                false => self.blockchain.get_block_locators()
            };
        }

        {
            let mut state = self.state.write();
            // Reset block counters.
            state.num_blocks_extending = 0;
            state.num_blocks_forking = 0;
        }

        // Request blocks from peer.
        self.inv_agent.get_blocks(
            locators,
            ConsensusAgent::GET_BLOCKS_MAX_RESULTS,
            ConsensusAgent::GET_BLOCKS_TIMEOUT);
    }

    fn on_inventory_event(&self, event: &InventoryEvent) {
        match event {
            InventoryEvent::KnownBlockAnnounced(hash) => self.on_known_block_announced(hash),
            InventoryEvent::NoNewObjectsAnnounced => self.on_no_new_objects_announced(),
            InventoryEvent::AllObjectsReceived => self.on_all_objects_received(),
            InventoryEvent::BlockProcessed(hash, result) => self.on_block_processed(hash, result),
            InventoryEvent::GetBlocksTimeout => self.on_get_blocks_timeout(),
            _ => {}
        }
    }

    fn on_known_block_announced(&self, hash: &Blake2bHash) {
        let mut state = self.state.write();
        if state.syncing {
            state.num_blocks_forking += 1;
            state.fork_head = Some(hash.clone());
        }
    }

    fn on_no_new_objects_announced(&self) {
        if self.state.read().syncing {
            self.perform_sync();
        }
    }

    fn on_all_objects_received(&self) {
        if self.state.read().syncing {
            self.perform_sync();
        }
    }

    fn on_block_processed(&self, hash: &Blake2bHash, result: &PushResult) {
        match result {
            PushResult::Invalid(_) => {
                self.peer.channel.close(CloseType::InvalidBlock);
            },
            PushResult::Extended | PushResult::Rebranched => {
                let mut state = self.state.write();
                if state.syncing {
                    state.num_blocks_extending += 1;
                }
            },
            PushResult::Forked => {
                let mut state = self.state.write();
                if state.syncing {
                    state.num_blocks_forking += 1;
                    state.fork_head = Some(hash.clone());
                }
            }
            PushResult::Orphan => {
                self.on_orphan_block(hash);
            }
            PushResult::Known => {
                debug!("Known block {} from {}", hash, self.peer.peer_address());
            }
        }
    }

    fn on_orphan_block(&self, hash: &Blake2bHash) {
        debug!("Orphan block {} from {}", hash, self.peer.peer_address());
        // TODO
    }

    fn on_get_blocks_timeout(&self) {
        self.peer.channel.close(CloseType::GetBlocksTimeout);
    }
}
