use std::collections::HashMap;
use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Weak};
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::task::{Context, Poll, Waker};
use std::thread;
use std::time::Instant;

use parking_lot::RwLock;

use beserial::Serialize;
use blockchain_base::{AbstractBlockchain, BlockchainEvent};
use database::Environment;
use database::ReadTransaction;
use hash::Blake2bHash;
use macros::upgrade_weak;
use utils::mutable_once::MutableOnce;

pub type SerializedChunk = Vec<u8>;

pub struct AccountsChunkCache<B: AbstractBlockchain + 'static> {
    blockchain: Arc<B>,
    env: Environment,
    computing_enabled: AtomicBool,
    chunks_by_prefix_by_block: RwLock<HashMap<Blake2bHash, HashMap<String, SerializedChunk>>>,
    wakers_by_block: RwLock<HashMap<Blake2bHash, Vec<Waker>>>,
    block_history_order: RwLock<VecDeque<Blake2bHash>>,
    weak_self: MutableOnce<Weak<Self>>
}

impl<B: AbstractBlockchain + 'static> AccountsChunkCache<B> {

    const MAX_BLOCKS_BACKLOG: usize = 10;
    const CHUNK_SIZE_MAX: usize = 5000;

    pub fn new(env: Environment, blockchain: Arc<B>) -> Arc<Self> {
        let cache = AccountsChunkCache {
            blockchain,
            env,
            computing_enabled: AtomicBool::new(false),
            chunks_by_prefix_by_block: RwLock::new(HashMap::with_capacity(Self::MAX_BLOCKS_BACKLOG + 1)),
            wakers_by_block: RwLock::new(HashMap::with_capacity(Self::MAX_BLOCKS_BACKLOG + 1)),
            block_history_order: RwLock::new(VecDeque::with_capacity(Self::MAX_BLOCKS_BACKLOG + 1)),
            weak_self: MutableOnce::new(Weak::new()),
        };
        let cache_arc = Arc::new(cache);
        unsafe { cache_arc.weak_self.replace(Arc::downgrade(&cache_arc)) };

        let cache_arc2 = Arc::clone(&cache_arc);
        cache_arc.blockchain.register_listener(move |event: &BlockchainEvent<B::Block>| cache_arc2.on_blockchain_event(event));

        cache_arc
    }

    /// Request a chunk from the cache.
    pub fn get_chunk(&self, hash: &Blake2bHash, prefix: &str) -> GetChunkFuture<B> {
        // Start computing of chunks on the first get_chunk request.
        // Swap should ensure that this is only triggered *once* and that no race condition can occur.
        if !self.computing_enabled.swap(true, Ordering::AcqRel) {
            self.compute_chunks_for_block();
        }
        GetChunkFuture::new(hash.clone(), prefix.to_string(), self.weak_self.upgrade().unwrap())
    }

    /// Trigger computation of chunks asynchronously after blockchain events.
    fn on_blockchain_event(&self, event: &BlockchainEvent<B::Block>) {
        if !self.computing_enabled.load(Ordering::Acquire) {
            // Only pre-compute chunks after a chunk was requested for the first time
            return;
        }
        match event {
            BlockchainEvent::Extended(_) | BlockchainEvent::Rebranched(_, _) => {
                self.compute_chunks_for_block();
            },
            BlockchainEvent::Finalized(_) => (),
        }
    }

    /// Internal function to asynchronously triggering the computation and caching of chunks.
    /// This function assumes to be called at most *once* per block hash.
    fn compute_chunks_for_block(&self) {
        let weak = self.weak_self.clone();
        thread::spawn(move || {
            let this: Arc<Self> = upgrade_weak!(weak);
            let txn = ReadTransaction::new(&this.env);
            let hash = match this.blockchain.head_hash_from_store(&txn) {
                Some(hash) => hash,
                None => return,
            };

            // Check that this hash is not yet worked on.
            {
                let mut guard = this.wakers_by_block.write();
                if guard.contains_key(&hash) {
                    return;
                }
                // Requests for accounts tree chunks of `block` are accepted now.
                guard.insert(hash.clone(), Vec::new());
            }

            trace!("Computing chunks for block {}", hash);

            // Compute and store chunks.
            let chunk_start = Instant::now();
            this.chunks_by_prefix_by_block.write().insert(hash.clone(), HashMap::new());
            let mut prefix = "".to_string();
            while let Some(chunk) = this.blockchain.get_accounts_chunk(&prefix[..], Self::CHUNK_SIZE_MAX, Some(&txn)) {
                if let Some(chunks_by_prefix) = this.chunks_by_prefix_by_block.write().get_mut(&hash) {
                    let last_terminal_string_opt = chunk.last_terminal_string();
                    let chunk_len = chunk.len();
                    chunks_by_prefix.insert(prefix.clone(), chunk.serialize_to_vec());
                    if chunk_len == 1 {
                        break;
                    }
                    if let Some(last_terminal_string) = last_terminal_string_opt {
                        prefix = last_terminal_string;
                    }
                } else {
                    break;
                }
                this.notify_wakers_for_block(&hash);
            }
            this.notify_wakers_for_block(&hash);
            // The chunks are cached, so newly created requests will not need to enter them into the list of tasks.
            this.wakers_by_block.write().remove(&hash.clone());

            let num_chunks = this.chunks_by_prefix_by_block.read().get(&hash).map_or(0, HashMap::len);
            trace!("Computing {} chunks for block {} tree took {:?}", num_chunks, hash, chunk_start.elapsed());

            // Put those blocks that are cached into a history, so that we can remove them later on.
            this.block_history_order.write().push_back(hash.clone());

            // Remove old chunks after some time.
            if this.block_history_order.read().len() > Self::MAX_BLOCKS_BACKLOG {
                // Take the oldest block to remove.
                if let Some(block_hash) = this.block_history_order.write().pop_front() {
                    // First clean up the chunks.
                    this.chunks_by_prefix_by_block.write().remove(&block_hash);

                    // Then remove the tasks (if present) and notify those that there won't be an update.
                    if let Some(wakers) = this.wakers_by_block.write().remove(&block_hash) {
                        for waker in wakers {
                            waker.wake();
                        }
                    }
                }
            }
        });
    }

    /// Notifies wakers that something changed.
    fn notify_wakers_for_block(&self, hash: &Blake2bHash) {
        if let Some(wakers) = self.wakers_by_block.read().get(&hash) {
            for waker in wakers {
                waker.clone().wake();
            }
        }
    }
}

/// Future given to the requester in order to retrieve accounts tree chunk.
pub struct GetChunkFuture<B: AbstractBlockchain + 'static> {
    hash: Blake2bHash,
    prefix: String,
    chunk_cache: Arc<AccountsChunkCache<B>>,
    running: bool
}

impl<B: AbstractBlockchain> GetChunkFuture<B> {
    pub fn new(hash: Blake2bHash, prefix: String, chunk_cache: Arc<AccountsChunkCache<B>>) -> Self {
        GetChunkFuture { hash, prefix, chunk_cache, running: false }
    }
}

impl<B: AbstractBlockchain> Future for GetChunkFuture<B> {
    type Output = Option<SerializedChunk>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = &mut *self;
        // If chunk is available, deliver it to the client.
        if let Some(chunks_by_prefix) = this.chunk_cache.chunks_by_prefix_by_block.read().get(&this.hash) {
            if let Some(chunk) = chunks_by_prefix.get(&this.prefix) {
                return Poll::Ready(Some(chunk.clone()));
            }
        }
        // Otherwise check if the task is (still) filed.
        if let Some(wakers) = this.chunk_cache.wakers_by_block.write().get_mut(&this.hash) {
            // If yes, "subscribe" to updates.
            if !this.running {
                wakers.push(cx.waker().clone());
                this.running = true;
            }
            Poll::Pending
        } else {
            // Else, the block is unknown, too far in the past or something else.
            // Return None in these cases.
            Poll::Ready(None)
        }
    }
}
