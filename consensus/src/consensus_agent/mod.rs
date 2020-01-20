use std::sync::{Arc, Weak};
use std::time::Duration;

use parking_lot::Mutex;
use parking_lot::MutexGuard;
use parking_lot::RwLock;
use rand::Rng;

use beserial::Serialize;
use block_base::Block;
use blockchain_base::{AbstractBlockchain, PushError, PushResult};
use hash::Blake2bHash;
use macros::upgrade_weak;
use mempool::{Mempool, ReturnCode};
use network::connection::close_type::CloseType;
use network::Peer;
use network_messages::{
    GetBlockProofMessage,
    GetBlocksMessage,
    GetEpochTransactionsMessage,
    MessageType,
    RejectMessage,
    RejectMessageCode,
};
use network_primitives::subscription::Subscription;
use primitives::coin::Coin;
use transaction::Transaction;
use utils::mutable_once::MutableOnce;
use utils::observer::{Notifier, weak_listener, weak_passthru_listener};
use utils::rate_limit::RateLimit;
use utils::timers::Timers;

use crate::accounts_chunk_cache::AccountsChunkCache;
use crate::consensus_agent::sync::{SyncProtocol, BlockQueue};
use crate::ConsensusProtocol;
use crate::inventory::{InventoryAgent, InventoryEvent, InventoryManager};

pub mod requests;
pub mod sync;

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ConsensusAgentEvent {
    Synced,
    OutOfSync,
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

    /// Rate limit for GetChainProof messages.
    chain_proof_limit: RateLimit,

    /// Rate limit for GetBlockProof messages.
    block_proof_limit: RateLimit,

    /// Rate limit for GetTransactionReceipts messages.
    transaction_receipts_limit: RateLimit,

    /// Rate limit for GetTransactionsProof messages.
    transactions_proof_limit: RateLimit,

    /// Rate limit for AccountsProof messages.
    accounts_proof_limit: RateLimit,

    /// Rate limit for GetEpochTransactions messages.
    epoch_transactions_limit: RateLimit,
}

#[derive(Ord, PartialOrd, PartialEq, Eq, Hash, Clone, Copy, Debug)]
enum ConsensusAgentTimer {
    Mempool,
    ResyncThrottle,
}


pub struct ConsensusAgent<P: ConsensusProtocol + 'static> {
    pub(crate) blockchain: Arc<P::Blockchain>,
    accounts_chunk_cache: Arc<AccountsChunkCache<P::Blockchain>>,
    pub peer: Arc<Peer>,

    inv_agent: Arc<InventoryAgent<P>>,
    sync_protocol: Arc<P::SyncProtocol>,

    pub(crate) state: RwLock<ConsensusAgentState>,

    pub notifier: RwLock<Notifier<'static, ConsensusAgentEvent>>,
    self_weak: MutableOnce<Weak<ConsensusAgent<P>>>,

    sync_lock: Mutex<()>,

    timers: Timers<ConsensusAgentTimer>,
}

impl<P: ConsensusProtocol + 'static> ConsensusAgent<P> {
    const SYNC_ATTEMPTS_MAX: u32 = 25;
    const GET_BLOCKS_TIMEOUT: Duration = Duration::from_secs(10);
    const GET_BLOCKS_MAX_RESULTS: u16 = 500;
    const RESYNC_THROTTLE: Duration = Duration::from_millis(10);

    const CHAIN_PROOF_RATE_LIMIT: usize = 3; // per minute
    const BLOCK_PROOF_RATE_LIMIT: usize = 60; // per minute
    const TRANSACTION_RECEIPTS_RATE_LIMIT: usize = 30; // per minute
    const TRANSACTIONS_PROOF_RATE_LIMIT: usize = 60; // per minute
    const ACCOUNTS_PROOF_RATE_LIMIT: usize = 60; // per minute
    const EPOCH_TRANSACTIONS_RATE_LIMIT: usize = 100; // per minute

    /// Minimum time to wait before triggering the initial mempool request.
    const MEMPOOL_DELAY_MIN: u64 = 2 * 1000; // in ms
    /// Maximum time to wait before triggering the initial mempool request.
    const MEMPOOL_DELAY_MAX: u64 = 20 * 1000; // in ms

    pub fn new(blockchain: Arc<P::Blockchain>, mempool: Arc<Mempool<P::Blockchain>>, inv_mgr: Arc<RwLock<InventoryManager<P>>>, accounts_chunk_cache: Arc<AccountsChunkCache<P::Blockchain>>, block_queue: Arc<RwLock<BlockQueue<P::Blockchain>>>, peer: Arc<Peer>) -> Arc<Self> {
        let sync_target = peer.head_hash.clone();
        let peer_arc = peer;
        let sync_protocol = <P::SyncProtocol as SyncProtocol<P::Blockchain>>::new(blockchain.clone(), block_queue, peer_arc.clone());
        let inv_agent = InventoryAgent::new(blockchain.clone(), mempool.clone(), inv_mgr, peer_arc.clone(), sync_protocol.clone());
        let this = Arc::new(ConsensusAgent {
            blockchain,
            accounts_chunk_cache,
            peer: peer_arc.clone(),
            inv_agent,
            sync_protocol,

            state: RwLock::new(ConsensusAgentState {
                syncing: false,
                synced: false,
                sync_target,
                fork_head: None,
                // Initialize to 1 to not count the initial sync call as a failed attempt.
                num_blocks_extending: 1,
                num_blocks_forking: 0,
                failed_syncs: 0,

                chain_proof_limit: RateLimit::new_per_minute(Self::CHAIN_PROOF_RATE_LIMIT),
                block_proof_limit: RateLimit::new_per_minute(Self::BLOCK_PROOF_RATE_LIMIT),
                transaction_receipts_limit: RateLimit::new_per_minute(Self::TRANSACTION_RECEIPTS_RATE_LIMIT),
                transactions_proof_limit: RateLimit::new_per_minute(Self::TRANSACTIONS_PROOF_RATE_LIMIT),
                accounts_proof_limit: RateLimit::new_per_minute(Self::ACCOUNTS_PROOF_RATE_LIMIT),
                epoch_transactions_limit: RateLimit::new_per_minute(Self::EPOCH_TRANSACTIONS_RATE_LIMIT),
            }),

            notifier: RwLock::new(Notifier::new()),
            self_weak: MutableOnce::new(Weak::new()),

            sync_lock: Mutex::new(()),
            timers: Timers::new()
        });
        ConsensusAgent::init_listeners(&this);
        this
    }

    fn init_listeners(this: &Arc<Self>) {
        unsafe { this.self_weak.replace(Arc::downgrade(this)) };

        this.inv_agent.notifier.write().register(weak_listener(
            Arc::downgrade(this),
            |this, e| this.on_inventory_event(e)));

        let msg_notifier = &this.peer.channel.msg_notifier;
        msg_notifier.get_chain_proof.write().register(weak_passthru_listener(
            Arc::downgrade(this),
            |this, _| this.on_get_chain_proof()));
        msg_notifier.get_block_proof.write().register(weak_passthru_listener(
            Arc::downgrade(this),
            |this, msg| this.on_get_block_proof(msg)));
        msg_notifier.get_transaction_receipts.write().register(weak_passthru_listener(
            Arc::downgrade(this),
            |this, msg| this.on_get_transaction_receipts(msg)));
        msg_notifier.get_transactions_proof.write().register(weak_passthru_listener(
            Arc::downgrade(this),
            |this, msg| this.on_get_transactions_proof(msg)));
        msg_notifier.get_accounts_proof.write().register(weak_passthru_listener(
            Arc::downgrade(this),
            |this, msg| this.on_get_accounts_proof(msg)));
        msg_notifier.get_accounts_tree_chunk.write().register(weak_passthru_listener(
            Arc::downgrade(this),
            |this, msg| this.on_get_accounts_tree_chunk(msg)));
        msg_notifier.get_epoch_transactions.write().register(weak_passthru_listener(
            Arc::downgrade(this),
            |this, msg: GetEpochTransactionsMessage| this.on_get_epoch_transactions(msg)));
    }

    pub fn relay_block(&self, block: &<P::Blockchain as AbstractBlockchain>::Block) -> bool {
        // Don't relay block if have not synced with the peer yet.
        if !self.state.read().synced {
            return false;
        }

        self.inv_agent.relay_block(block)
    }

    pub fn relay_transaction(&self, transaction: &Transaction) -> bool {
        self.inv_agent.relay_transaction(transaction)
    }

    pub fn remove_transaction(&self, transaction: &Transaction) {
        self.inv_agent.remove_transaction(transaction);
    }

    pub fn synced(&self) -> bool {
        self.state.read().synced
    }

    pub fn sync(&self) {
        self.state.write().syncing = true;
        self.sync_protocol.initiate_sync();

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
                if state.failed_syncs >= Self::SYNC_ATTEMPTS_MAX {
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
        let weak = self.self_weak.clone();
        self.timers.set_delay(ConsensusAgentTimer::Mempool, move || {
            let agent = upgrade_weak!(weak);
            agent.timers.clear_delay(&ConsensusAgentTimer::Mempool);
            agent.inv_agent.mempool();
        }, Duration::from_millis(rand::thread_rng()
            .gen_range(Self::MEMPOOL_DELAY_MIN, Self::MEMPOOL_DELAY_MAX)));


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

            locators = if on_fork {
                vec![state.fork_head.as_ref().unwrap().clone()]
            } else {
                self.sync_protocol.get_block_locators(GetBlocksMessage::LOCATORS_MAX_COUNT)
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
            Self::GET_BLOCKS_MAX_RESULTS,
            Self::GET_BLOCKS_TIMEOUT);
    }

    fn on_get_chain_proof(&self) {
        // TODO: Implement
        // XXX This will be used here, so remove the warning
        let _ = self.state.read().chain_proof_limit;
    }

    fn on_get_block_proof(&self, _message: GetBlockProofMessage) {
        // TODO: Implement
        // XXX This will be used here, so remove the warning
        let _ = self.state.read().block_proof_limit;
    }

    fn on_inventory_event(&self, event: &InventoryEvent<<<P::Blockchain as AbstractBlockchain>::Block as Block>::Error>) {
        match event {
            InventoryEvent::NewBlockAnnounced(hash, head_candidate) => self.on_new_block_announced(hash, *head_candidate),
            InventoryEvent::KnownBlockAnnounced(hash) => self.on_known_block_announced(hash),
            InventoryEvent::NoNewObjectsAnnounced => self.on_no_new_objects_announced(),
            InventoryEvent::AllObjectsReceived => self.on_all_objects_received(),
            InventoryEvent::BlockProcessed(hash, result) => self.on_block_processed(hash, result),
            InventoryEvent::TransactionProcessed(hash, result) => self.on_tx_processed(hash, result),
            InventoryEvent::GetBlocksTimeout => self.on_get_blocks_timeout(),
            _ => {}
        }
    }

    fn on_new_block_announced(&self, hash: &Blake2bHash, head_candidate: bool) {
        let mut state = self.state.write();
        if !state.synced && head_candidate {
            state.sync_target = hash.clone();
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

    fn on_block_processed(&self, hash: &Blake2bHash, result: &Result<PushResult, PushError<<<P::Blockchain as AbstractBlockchain>::Block as Block>::Error>>) {
        match result {
            Ok(PushResult::Extended) | Ok(PushResult::Rebranched) => {
                let mut state = self.state.write();
                if state.syncing {
                    state.num_blocks_extending += 1;
                }
            },
            Ok(PushResult::Forked) => {
                let mut state = self.state.write();
                if state.syncing {
                    state.num_blocks_forking += 1;
                    state.fork_head = Some(hash.clone());
                }
            },
            Ok(PushResult::Known) => {
                trace!("Known block {} from {}", hash, self.peer.peer_address());
            },
            Ok(PushResult::Ignored) => {
                // Stop syncing with this peer if the block has been ignored,
                // because it is an inferior chain.
                if self.state.read().syncing {
                    let sync_guard = self.sync_lock.lock();
                    self.sync_finished(sync_guard);
                }
            },
            Err(PushError::Orphan) => {
                self.on_orphan_block(hash);
            },
            Err(_) => {
                self.peer.channel.close(CloseType::InvalidBlock);
            },
        }
    }

    fn on_tx_processed(&self, hash: &Blake2bHash, result: &ReturnCode) {
        match result {
            ReturnCode::Accepted => {
                debug!("Accepted tx {} from {}", hash, self.peer.peer_address());
            },
            ReturnCode::Known => {
                debug!("Known tx {} from {}", hash, self.peer.peer_address());
            },
            ReturnCode::FeeTooLow => {
                self.peer.channel.send_or_close(RejectMessage::new(
                    MessageType::Tx,
                    RejectMessageCode::InsufficientFee,
                    String::from("Sender has too many free transactions)"),
                    Some(hash.serialize_to_vec())
                ));
            },
            ReturnCode::Invalid => {
                self.peer.channel.send_or_close(RejectMessage::new(
                    MessageType::Tx,
                    RejectMessageCode::Invalid,
                    String::from("Invalid transaction"),
                    Some(hash.serialize_to_vec())
                ));
            },
            ReturnCode::Filtered => {
                debug!("Filtered tx {} from {}", hash, self.peer.peer_address());
            },
        }
    }

    fn on_orphan_block(&self, hash: &Blake2bHash) {
        // Set the orphaned block as the new sync target.
        self.state.write().sync_target = hash.clone();

        // Ignore orphan blocks if we're not synced yet. This shouldn't happen.
        if !self.state.read().synced {
            debug!("Received orphan block {} from {} before/while syncing", hash, self.peer.peer_address());
            return;
        }

        // The peer has announced an orphaned block after the initial sync. We're probably out of sync.
        debug!("Received orphan block {} from {}", hash, self.peer.peer_address());

        // Disable transaction announcements from the peer once.
        if !self.timers.delay_exists(&ConsensusAgentTimer::ResyncThrottle) {
            self.inv_agent.subscribe(Subscription::MinFee(Coin::from_u64_unchecked(Coin::MAX_SAFE_VALUE)));
        }

        // Wait a short time for:
        // - our (un-)subscribe message to be sent
        // - potentially more orphaned blocks to arrive
        let weak = self.self_weak.clone();
        self.timers.reset_delay(ConsensusAgentTimer::ResyncThrottle, move || {
            let this = upgrade_weak!(weak);
            this.out_of_sync();
        }, Self::RESYNC_THROTTLE);
    }

    fn out_of_sync(&self) {
        self.timers.clear_delay(&ConsensusAgentTimer::ResyncThrottle);

        self.state.write().synced = false;

        self.notifier.read().notify(ConsensusAgentEvent::OutOfSync);
    }

    fn on_get_blocks_timeout(&self) {
        self.peer.channel.close(CloseType::GetBlocksTimeout);
    }
}
