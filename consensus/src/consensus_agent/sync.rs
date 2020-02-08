use std::collections::vec_deque::VecDeque;
use std::default::Default;
use std::mem;
use std::sync::{Arc, Weak};
use std::time::Duration;

use parking_lot::{RwLock, RwLockUpgradableReadGuard};

use block_albatross::Block as AlbatrossBlock;
use block_albatross::BlockError as AlbatrossBlockError;
use block_base::{Block, BlockError};
use blockchain_albatross::Blockchain as AlbatrossBlockchain;
use blockchain_base::{AbstractBlockchain, PushError, PushResult};
use collections::LimitHashSet;
use hash::Blake2bHash;
use macros::upgrade_weak;
use network::connection::close_type::CloseType;
use network::peer::Peer;
use network_messages::{EpochTransactionsMessage, GetBlocksDirection, GetBlocksMessage, GetEpochTransactionsMessage};
use primitives::policy;
use transaction::Transaction;
use utils::merkle::partial::PartialMerkleProofResult;
use utils::mutable_once::MutableOnce;
use utils::observer::{PassThroughListener, PassThroughNotifier, weak_listener};
use utils::timers::Timers;

pub trait SyncProtocol<B: AbstractBlockchain>: Send + Sync {
    fn new(blockchain: Arc<B>, peer: Arc<Peer>) -> Arc<Self>;
    fn initiate_sync(&self) {}
    fn get_block_locators(&self, max_count: usize) -> Vec<Blake2bHash>;
    fn request_blocks(&self, locators: Vec<Blake2bHash>, max_results: u16);
    fn on_block(&self, block: B::Block);
    fn on_epoch_transactions(&self, epoch_transactions: EpochTransactionsMessage);
    fn on_no_new_objects_announced(&self) {}
    fn on_all_objects_received(&self) {}
    fn register_listener<L: PassThroughListener<SyncEvent<<B::Block as Block>::Error>> + 'static>(&self, listener: L);
    fn deregister_listener(&self);
}

#[derive(Debug, PartialEq, Eq)]
pub enum SyncEvent<BE: BlockError> {
    BlockProcessed(Blake2bHash, Result<PushResult, PushError<BE>>),
}

pub struct FullSync<B: AbstractBlockchain> {
    blockchain: Arc<B>,
    peer: Arc<Peer>,
    notifier: RwLock<PassThroughNotifier<'static, SyncEvent<<B::Block as Block>::Error>>>,

    /// Inferior chain block hashes.
    ignored_blocks: RwLock<LimitHashSet<Blake2bHash>>,
}

impl<B: AbstractBlockchain> FullSync<B> {
    const IGNORED_BLOCKS_COUNT_MAX: usize = 40000;
}

impl<B: AbstractBlockchain> SyncProtocol<B> for FullSync<B> {
    fn new(blockchain: Arc<B>, peer: Arc<Peer>) -> Arc<Self> {
        Arc::new(Self {
            blockchain,
            peer,
            notifier: RwLock::new(PassThroughNotifier::new()),
            ignored_blocks: RwLock::new(LimitHashSet::new(Self::IGNORED_BLOCKS_COUNT_MAX)),
        })
    }

    fn get_block_locators(&self, max_count: usize) -> Vec<Blake2bHash> {
        self.blockchain.get_block_locators(max_count)
    }

    fn request_blocks(&self, locators: Vec<Blake2bHash>, max_results: u16) {
        self.peer.channel.send_or_close(GetBlocksMessage::new(
            locators,
            max_results,
            GetBlocksDirection::Forward,
        ));
    }

    fn on_block(&self, block: B::Block) {
        // TODO: Move this to a better location to avoid requesting of these blocks at all.
        // If the block builds on an ignored block, ignore this one as well and return.
        let mut ignored_blocks = self.ignored_blocks.write();
        if ignored_blocks.contains(block.prev_hash()) {
            ignored_blocks.insert(block.hash());
            info!("Ignoring block on inferior chain #{}", block.height());
            return;
        }
        drop(ignored_blocks);

        let hash = block.hash();
        let result = self.blockchain.push(block);
        if let Ok(PushResult::Ignored) = result {
            self.ignored_blocks.write().insert(hash.clone());
        }
        self.notifier.read().notify(SyncEvent::BlockProcessed(hash, result));
    }

    fn on_epoch_transactions(&self, _epoch_transactions: EpochTransactionsMessage) {
        warn!("We didn't expect any epoch transactions from {} - discarding and closing the channel", self.peer.peer_address());
        self.peer.channel.close(CloseType::UnexpectedEpochTransactions);
    }

    fn register_listener<L: PassThroughListener<SyncEvent<<B::Block as Block>::Error>> + 'static>(&self, listener: L) {
        self.notifier.write().register(listener)
    }

    fn deregister_listener(&self) {
        self.notifier.write().deregister()
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
enum MacroBlockSyncPhase {
    MacroBlocks,
    MicroBlocks,
    Finished,
}

struct MacroBlockSyncState {
    /// Cache of the next max. 1000 blocks.
    block_cache: VecDeque<AlbatrossBlock>,
    /// Transactions of the current block.
    transactions_cache: Vec<Transaction>,
    /// The current state of the syncing.
    phase: MacroBlockSyncPhase,
    /// Boolean flag whether we are currently processing an epoch.
    processing_epoch: bool,
    /// Previous proof's result.
    previous_result: Option<PartialMerkleProofResult<Blake2bHash>>,
}

impl Default for MacroBlockSyncState {
    fn default() -> Self {
        Self {
            block_cache: VecDeque::new(),
            transactions_cache: Vec::new(),
            phase: MacroBlockSyncPhase::Finished,
            processing_epoch: false,
            previous_result: None,
        }
    }
}

impl MacroBlockSyncState {
    fn new() -> Self {
        Default::default()
    }
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum MacroBlockSyncTimer {
    EpochTransactions(u32)
}

pub struct MacroBlockSync {
    blockchain: Arc<AlbatrossBlockchain>,
    state: RwLock<MacroBlockSyncState>,
    peer: Arc<Peer>,
    notifier: RwLock<PassThroughNotifier<'static, SyncEvent<AlbatrossBlockError>>>,
    timers: Timers<MacroBlockSyncTimer>,
    self_weak: MutableOnce<Weak<MacroBlockSync>>,
}

impl MacroBlockSync {
    /// Maximum time to wait after sending out get-data or receiving the last object for this request.
    const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

    fn complete_epoch(&self, block: AlbatrossBlock, transactions: &[Transaction]) {
        self.state.write().processing_epoch = false;

        let hash = block.hash();
        let result = self.blockchain.push_isolated_macro_block(block, transactions);
        self.notifier.read().notify(SyncEvent::BlockProcessed(hash, result));

        self.start_processing();
    }

    fn start_processing(&self) {
        let mut state = self.state.write();

        if !state.processing_epoch && !state.block_cache.is_empty() {
            state.processing_epoch = true;
            let block = state.block_cache.front().unwrap();
            let epoch = policy::epoch_at(block.block_number());

            // Set timeout.
            let weak = self.self_weak.clone();
            self.timers.set_delay(MacroBlockSyncTimer::EpochTransactions(epoch), move || {
                let this = upgrade_weak!(weak);
                // TODO: What should happen if we don't receive any response?
                // For now, just drop the connection with that peer.
                this.peer.channel.close(CloseType::GetEpochTransactionsTimeout);
            }, Self::REQUEST_TIMEOUT);

            self.peer.channel.send_or_close(GetEpochTransactionsMessage::new(epoch));
        }
    }

    fn on_close(&self) {
        self.timers.clear_all();
    }
}

impl SyncProtocol<AlbatrossBlockchain> for MacroBlockSync {
    fn new(blockchain: Arc<AlbatrossBlockchain>, peer: Arc<Peer>) -> Arc<Self> {
        let this = Arc::new(Self {
            peer,
            blockchain,
            state: RwLock::new(MacroBlockSyncState::new()),
            notifier: RwLock::new(PassThroughNotifier::new()),
            timers: Timers::new(),
            self_weak: MutableOnce::new(Weak::new()),
        });

        // Update the self weak reference.
        unsafe {
            let weak = Arc::downgrade(&this);
            this.self_weak.replace(weak);
        }

        {
            let mut close_notifier = this.peer.channel.close_notifier.write();
            close_notifier.register(weak_listener(
                Arc::downgrade(&this),
                |this, _| this.on_close()));
        }

        this
    }

    fn initiate_sync(&self) {
        let mut state = self.state.write();
        if state.phase == MacroBlockSyncPhase::Finished {
            state.phase = MacroBlockSyncPhase::MacroBlocks;
        }
    }

    fn get_block_locators(&self, max_count: usize) -> Vec<Blake2bHash> {
        self.blockchain.get_macro_block_locators(max_count)
    }

    fn request_blocks(&self, locators: Vec<Blake2bHash>, max_results: u16) {
        let message = match self.state.read().phase {
            MacroBlockSyncPhase::MacroBlocks => GetBlocksMessage::new_with_macro(
                locators,
                max_results,
                GetBlocksDirection::Forward,
            ),
            _ => GetBlocksMessage::new(
                locators,
                max_results,
                GetBlocksDirection::Forward,
            ),
        };
        self.peer.channel.send_or_close(message);
    }

    fn on_block(&self, block: AlbatrossBlock) {
        let mut state = self.state.write();
        let hash = block.hash();
        match state.phase {
            MacroBlockSyncPhase::MacroBlocks => {
                // Cache block and request transactions.
                state.block_cache.push_back(block);
                drop(state);
                self.start_processing();
            },
            _ => {
                let result = self.blockchain.push(block);
                self.notifier.read().notify(SyncEvent::BlockProcessed(hash, result));
            }
        }
    }

    fn on_epoch_transactions(&self, epoch_transactions: EpochTransactionsMessage) {
        // Validate proof to prevent the peer from spamming us with transactions.
        let proof = epoch_transactions.tx_proof;
        let mut transactions = epoch_transactions.transactions;

        let state = self.state.upgradable_read();

        let expected_root;
        match state.block_cache.front() {
            Some(AlbatrossBlock::Macro(ref macro_block)) => {
                if policy::epoch_at(macro_block.header.block_number) != epoch_transactions.epoch {
                    warn!("We didn't expect any transactions for epoch {} from {} - discarding and closing the channel", epoch_transactions.epoch, self.peer.peer_address());
                    self.peer.channel.close(CloseType::UnexpectedEpochTransactions);
                    return;
                }

                expected_root = macro_block.header.transactions_root.clone();
            },
            None => {
                warn!("We didn't expect any transactions for epoch {} from {} - discarding and closing the channel", epoch_transactions.epoch, self.peer.peer_address());
                self.peer.channel.close(CloseType::UnexpectedEpochTransactions);
                return;
            },
            _ => unreachable!(),
        }

        match proof.compute_root_from_values(&transactions, state.previous_result.as_ref()) {
            Ok(result) => {
                let mut state = RwLockUpgradableReadGuard::upgrade(state);
                // Check that root corresponds to root for this epoch
                if result.root() != &expected_root {
                    warn!("We received transactions with an invalid proof for epoch {} from {} - discarding and closing the channel", epoch_transactions.epoch, self.peer.peer_address());
                    self.peer.channel.close(CloseType::InvalidEpochTransactions);
                    return;
                }

                // Append transactions.
                state.transactions_cache.append(&mut transactions);
                state.previous_result = Some(result);

                if proof.is_empty() {
                    self.timers.clear_delay(&MacroBlockSyncTimer::EpochTransactions(epoch_transactions.epoch));

                    let transactions = mem::replace(&mut state.transactions_cache, Vec::new());
                    state.previous_result = None;

                    let block = state.block_cache.pop_front().unwrap();

                    drop(state);
                    self.complete_epoch(block, &transactions);
                } else {
                    // Reset delay to allow for more time.
                    let weak = self.self_weak.clone();
                    self.timers.reset_delay(MacroBlockSyncTimer::EpochTransactions(epoch_transactions.epoch), move || {
                        let this = upgrade_weak!(weak);
                        // TODO: What should happen if we don't receive any response?
                        // For now, just drop the connection with that peer.
                        this.peer.channel.close(CloseType::GetEpochTransactionsTimeout);
                    }, Self::REQUEST_TIMEOUT);
                }
            },
            Err(e) => {
                warn!("We received an invalid merkle proof ({:?}) from {} - discarding and closing the channel", e, self.peer.peer_address());
                self.peer.channel.close(CloseType::InvalidEpochTransactions);
                return;
            },
        }
    }

    fn on_no_new_objects_announced(&self) {
        let mut state = self.state.write();
        match state.phase {
            MacroBlockSyncPhase::MacroBlocks => {
                state.phase = MacroBlockSyncPhase::MicroBlocks;
            },
            MacroBlockSyncPhase::MicroBlocks => {
                state.phase = MacroBlockSyncPhase::Finished;
            },
            _ => {},
        }
    }

    fn register_listener<L: PassThroughListener<SyncEvent<<AlbatrossBlock as Block>::Error>> + 'static>(&self, listener: L) {
        self.notifier.write().register(listener)
    }

    fn deregister_listener(&self) {
        self.notifier.write().deregister()
    }
}

