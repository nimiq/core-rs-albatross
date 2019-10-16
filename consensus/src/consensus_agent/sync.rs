use std::collections::vec_deque::VecDeque;
use std::mem;
use std::sync::Arc;

use parking_lot::{RwLock, RwLockUpgradableReadGuard};

use block_albatross::Block as AlbatrossBlock;
use block_albatross::BlockError as AlbatrossBlockError;
use block_base::{Block, BlockError};
use blockchain_albatross::Blockchain as AlbatrossBlockchain;
use blockchain_base::{AbstractBlockchain, PushError, PushResult};
use hash::{Blake2bHash, Blake2bHasher};
use network::connection::close_type::CloseType;
use network::peer::Peer;
use network_messages::{EpochTransactionsMessage, GetBlocksDirection, GetBlocksMessage, GetEpochTransactionsMessage};
use primitives::policy;
use transaction::Transaction;
use utils::merkle::compute_root_from_content;
use utils::observer::{PassThroughListener, PassThroughNotifier};

pub trait SyncProtocol<'env, B: AbstractBlockchain<'env>>: Send + Sync {
    fn new(blockchain: Arc<B>, peer: Arc<Peer>) -> Self;
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

pub struct FullSync<'env, B: AbstractBlockchain<'env>> {
    blockchain: Arc<B>,
    peer: Arc<Peer>,
    notifier: RwLock<PassThroughNotifier<'static, SyncEvent<<B::Block as Block>::Error>>>,
}

impl<'env, B: AbstractBlockchain<'env>> SyncProtocol<'env, B> for FullSync<'env, B> {
    fn new(blockchain: Arc<B>, peer: Arc<Peer>) -> Self {
        Self {
            blockchain,
            peer,
            notifier: RwLock::new(PassThroughNotifier::new()),
        }
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
        let hash = block.hash();
        let result = self.blockchain.push(block);
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
enum MacroBlockSyncingState {
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
    state: MacroBlockSyncingState,

}

impl MacroBlockSyncState {
    fn new() -> Self {
        Self {
            block_cache: VecDeque::new(),
            transactions_cache: Vec::new(),
            state: MacroBlockSyncingState::Finished,
        }
    }
}

pub struct MacroBlockSync<'env> {
    blockchain: Arc<AlbatrossBlockchain<'env>>,
    state: RwLock<MacroBlockSyncState>,
    peer: Arc<Peer>,
    notifier: RwLock<PassThroughNotifier<'static, SyncEvent<AlbatrossBlockError>>>,
}

impl<'env> MacroBlockSync<'env> {
    fn complete_epoch(&self, block: AlbatrossBlock, transactions: &[Transaction]) {
        let hash = block.hash();
        let result = self.blockchain.push_isolated_macro_block(block, transactions);
        self.notifier.read().notify(SyncEvent::BlockProcessed(hash, result));
    }
}

impl<'env> SyncProtocol<'env, AlbatrossBlockchain<'env>> for MacroBlockSync<'env> {
    fn new(blockchain: Arc<AlbatrossBlockchain<'env>>, peer: Arc<Peer>) -> Self {
        Self {
            peer,
            blockchain,
            state: RwLock::new(MacroBlockSyncState::new()),
            notifier: RwLock::new(PassThroughNotifier::new()),
        }
    }

    fn initiate_sync(&self) {
        let mut state = self.state.write();
        if state.state == MacroBlockSyncingState::Finished {
            state.state = MacroBlockSyncingState::MacroBlocks;
        }
    }

    fn get_block_locators(&self, max_count: usize) -> Vec<Blake2bHash> {
        self.blockchain.get_macro_block_locators(max_count)
    }

    fn request_blocks(&self, locators: Vec<Blake2bHash>, max_results: u16) {
        let message = match self.state.read().state {
            MacroBlockSyncingState::MacroBlocks => GetBlocksMessage::new_with_macro(
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
        match state.state {
            MacroBlockSyncingState::MacroBlocks => {
                // Cache block and request transactions.
                state.block_cache.push_back(block);
                // TODO: Handle timeout.
                self.peer.channel.send_or_close(GetEpochTransactionsMessage::new(hash));
            },
            _ => {
                let result = self.blockchain.push(block);
                self.notifier.read().notify(SyncEvent::BlockProcessed(hash, result));
            }
        }
    }

    fn on_epoch_transactions(&self, epoch_transactions: EpochTransactionsMessage) {
        // Validate proof to prevent the peer from spamming us with transactions.
        let proof = epoch_transactions.tx_proof.proof;
        let transactions = epoch_transactions.tx_proof.transactions;

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

        match proof.compute_root_from_values(&transactions) {
            Ok(root) => {
                let mut state = RwLockUpgradableReadGuard::upgrade(state);
                // Check that root corresponds to root for this epoch
                if root != expected_root {
                    warn!("We received transactions with an invalid proof for epoch {} from {} - discarding and closing the channel", epoch_transactions.epoch, self.peer.peer_address());
                    self.peer.channel.close(CloseType::InvalidEpochTransactions);
                    return;
                }

                // Append transactions
                for tx in transactions {
                    state.transactions_cache.push(tx);
                }

                if epoch_transactions.last {
                    let transactions = mem::replace(&mut state.transactions_cache, Vec::new());
                    // Check full root
                    let full_root: Blake2bHash = compute_root_from_content::<Blake2bHasher, _>(&transactions);

                    if full_root != expected_root {
                        warn!("We received transactions with an invalid hash for epoch {} from {} - discarding and closing the channel", epoch_transactions.epoch, self.peer.peer_address());
                        self.peer.channel.close(CloseType::InvalidEpochTransactions);
                        return;
                    }

                    let block = state.block_cache.pop_front().unwrap();

                    drop(state);
                    self.complete_epoch(block, &transactions);
                }
            },
            Err(e) => {
                warn!("We received an invalid merkle proof from {} - discarding and closing the channel", self.peer.peer_address());
                self.peer.channel.close(CloseType::InvalidEpochTransactions);
                return;
            },
        }
    }

    fn on_no_new_objects_announced(&self) {
        let mut state = self.state.write();
        match state.state {
            MacroBlockSyncingState::MacroBlocks => {
                state.state = MacroBlockSyncingState::MicroBlocks;
            },
            MacroBlockSyncingState::MicroBlocks => {
                state.state = MacroBlockSyncingState::Finished;
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

