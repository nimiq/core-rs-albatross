use std::collections::HashSet;
use std::sync::Arc;
use crate::consensus::base::blockchain::Blockchain;
use crate::consensus::base::mempool::Mempool;
use crate::consensus::base::primitive::hash::Blake2bHash;
use crate::network::Peer;
use crate::network::message::InvVector;
use crate::consensus::inventory::InventoryManager;
use parking_lot::RwLock;
use crate::consensus::inventory::InventoryAgent;
use crate::network::message::GetBlocksMessage;
use crate::network::message::GetBlocksDirection;
use crate::network::connection::close_type::CloseType;
use crate::utils::observer::Notifier;
use std::sync::Weak;
use crate::consensus::inventory::InventoryEvent;
use crate::utils::observer::weak_listener;
use crate::consensus::base::blockchain::PushResult;
use crate::utils::timers::Timers;
use std::time::Duration;
use futures::future;
use futures::future::Future;
use tokio::timer::Delay;
use std::time::Instant;

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ConsensusAgentEvent {
    Synced
}

pub struct ConsensusAgent {
    blockchain: Arc<Blockchain<'static>>,
    mempool: Arc<Mempool<'static>>,
    peer: Arc<Peer>,

    inv_agent: Arc<InventoryAgent>,

    /// Flag indicating that we are currently syncing our blockchain with the peer's.
    syncing: bool,

    /// Flag indicating that we have synced our blockchain with the peer's.
    synced: bool,

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

    notifier: Notifier<'static, ConsensusAgentEvent>,
    self_weak: Weak<RwLock<ConsensusAgent>>,
}

impl ConsensusAgent {
    const SYNC_ATTEMPTS_MAX: u32 = 25;
    const GET_BLOCKS_TIMEOUT: Duration = Duration::from_secs(10);
    const GET_BLOCKS_MAX_RESULTS: u16 = 500;

    pub fn new(blockchain: Arc<Blockchain<'static>>, mempool: Arc<Mempool<'static>>, inv_mgr: Arc<RwLock<InventoryManager>>, peer: Peer) -> Arc<RwLock<Self>> {
        let sync_target = peer.head_hash.clone();
        let peer_arc = Arc::new(peer);
        let inv_agent = InventoryAgent::new(blockchain.clone(), mempool.clone(), inv_mgr,peer_arc.clone());
        let this = Arc::new(RwLock::new(ConsensusAgent {
            blockchain,
            mempool,
            peer: peer_arc.clone(),
            inv_agent,

            syncing: false,
            synced: false,
            sync_target,
            fork_head: None,
            // Initialize to 1 to not count the initial sync call as a failed attempt.
            num_blocks_extending: 1,
            num_blocks_forking: 0,
            failed_syncs: 0,

            notifier: Notifier::new(),
            self_weak: Weak::new(),
        }));
        ConsensusAgent::init_listeners(&this);
        this
    }

    fn init_listeners(this: &Arc<RwLock<ConsensusAgent>>) {
        this.write().self_weak = Arc::downgrade(this);

        let weak = Arc::downgrade(this);
        let agent = this.read();
        agent.inv_agent.notifier.write().register(move |e: &InventoryEvent| {
            let this = upgrade_weak!(weak);
            this.write().on_inventory_event(e);
        });
    }

    pub fn sync(&mut self) {
        self.syncing = true;

        // Don't go through the InventoryManager when syncing.
        self.inv_agent.bypass_mgr(true);

        self.perform_sync();
    }

    fn perform_sync(&mut self) {
        // Wait for ongoing requests to finish.
        if self.inv_agent.is_busy() {
            return;
        }

        // If we know our sync target block, the sync is finished.
        if self.blockchain.contains(&self.sync_target, true) {
            self.sync_finished();
            return;
        }

        // If the peer didn't send us any blocks that extended our chain, count it as a failed sync attempt.
        // This sets a maximum length for forks that the full client will accept:
        //   FullConsensusAgent.SYNC_ATTEMPTS_MAX * BaseInvectoryMessage.VECTORS_MAX_COUNT
        if self.num_blocks_extending == 0 {
            self.failed_syncs += 1;
            if self.failed_syncs >= ConsensusAgent::SYNC_ATTEMPTS_MAX {
                self.peer.channel.close(CloseType::BlockchainSyncFailed);
                return;
            }
        }

        // We don't know the peer's head block, request blocks from it.
        self.request_blocks();
    }

    fn sync_finished(&mut self) {
        // TODO Subscribe to all announcements from the peer.

        // TODO Request the peer's mempool.
        // XXX Use a random delay here to prevent requests to multiple peers at once.

        self.syncing = false;
        self.synced = true;

        self.num_blocks_extending = 1;
        self.num_blocks_forking = 0;
        self.fork_head = None;
        self.failed_syncs = 0;

        self.notifier.notify(ConsensusAgentEvent::Synced);
    }

    fn request_blocks(&mut self) {
        // Check if the peer is sending us a fork.
        let on_fork = self.fork_head.is_some() && self.num_blocks_extending == 0 && self.num_blocks_forking > 0;

        let locators = match on_fork {
            true => vec![self.fork_head.as_ref().unwrap().clone()],
            false => self.blockchain.get_block_locators()
        };

        // Reset block counters.
        self.num_blocks_extending = 0;
        self.num_blocks_forking = 0;

        // Request blocks from peer.
        self.inv_agent.get_blocks(
            locators,
            ConsensusAgent::GET_BLOCKS_MAX_RESULTS,
            ConsensusAgent::GET_BLOCKS_TIMEOUT);
    }

    fn on_inventory_event(&mut self, event: &InventoryEvent) {
        match event {
            InventoryEvent::KnownBlockAnnounced(hash) => self.on_known_block_announced(hash),
            InventoryEvent::NoNewObjectsAnnounced => self.on_no_new_objects_announced(),
            InventoryEvent::AllObjectsReceived => self.on_all_objects_received(),
            InventoryEvent::BlockProcessed(hash, result) => self.on_block_processed(hash, result),
            InventoryEvent::GetBlocksTimeout => self.on_get_blocks_timeout(),
            _ => {}
        }
    }

    fn on_known_block_announced(&mut self, hash: &Blake2bHash) {
        if self.syncing {
            self.num_blocks_forking += 1;
            self.fork_head = Some(hash.clone());
        }
    }

    fn on_no_new_objects_announced(&mut self) {
        if self.syncing {
            self.perform_sync();
        }
    }

    fn on_all_objects_received(&mut self) {
        if self.syncing {
            self.perform_sync();
        }
    }

    fn on_block_processed(&mut self, hash: &Blake2bHash, result: &PushResult) {
        match result {
            PushResult::Invalid(_) => {
                self.peer.channel.close(CloseType::InvalidBlock);
            },
            PushResult::Extended | PushResult::Rebranched => {
                if self.syncing {
                    self.num_blocks_extending += 1;
                }
            },
            PushResult::Forked => {
                if self.syncing {
                    self.num_blocks_forking += 1;
                    self.fork_head = Some(hash.clone());
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

    fn on_orphan_block(&mut self, hash: &Blake2bHash) {
        debug!("Orphan block {} from {}", hash, self.peer.peer_address());
        // TODO
    }

    fn on_get_blocks_timeout(&mut self) {
        self.peer.channel.close(CloseType::GetBlocksTimeout);
    }
}
