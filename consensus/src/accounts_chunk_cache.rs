use std::collections::HashMap;
use std::collections::VecDeque;
use std::sync::{Arc, Weak};
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::thread;
use std::time::Instant;

use futures::future::Future;
use futures::Poll;
use futures::prelude::*;
use futures::task;
use futures::task::Task;
use parking_lot::RwLock;

use beserial::Serialize;
use blockchain::{Blockchain, BlockchainEvent};
use database::Environment;
use database::ReadTransaction;
use hash::Blake2bHash;
use utils::mutable_once::MutableOnce;

pub type SerializedChunk = Vec<u8>;

pub struct AccountsChunkCache {
    blockchain: Arc<Blockchain<'static>>,
    env: &'static Environment,
    computing_enabled: AtomicBool,
    chunks_by_prefix_by_block: RwLock<HashMap<Blake2bHash, HashMap<String, SerializedChunk>>>,
    tasks_by_block: RwLock<HashMap<Blake2bHash, Vec<Task>>>,
    block_history_order: RwLock<VecDeque<Blake2bHash>>,
    weak_self: MutableOnce<Weak<Self>>
}

impl AccountsChunkCache {

    const MAX_BLOCKS_BACKLOG: usize = 10;
    const CHUNK_SIZE_MAX: usize = 5000;

    pub fn new(env: &'static Environment, blockchain: Arc<Blockchain<'static>>) -> Arc<Self> {
        let cache = AccountsChunkCache {
            blockchain,
            env,
            computing_enabled: AtomicBool::new(false),
            chunks_by_prefix_by_block: RwLock::new(HashMap::with_capacity(Self::MAX_BLOCKS_BACKLOG + 1)),
            tasks_by_block: RwLock::new(HashMap::with_capacity(Self::MAX_BLOCKS_BACKLOG + 1)),
            block_history_order: RwLock::new(VecDeque::with_capacity(Self::MAX_BLOCKS_BACKLOG + 1)),
            weak_self: MutableOnce::new(Weak::new()),
        };
        let cache_arc = Arc::new(cache);
        unsafe { cache_arc.weak_self.replace(Arc::downgrade(&cache_arc)) };

        let cache_arc2 = Arc::clone(&cache_arc);
        cache_arc.blockchain.notifier.write().register(move |event: &BlockchainEvent| cache_arc2.on_blockchain_event(event));

        cache_arc
    }

    /// Request a chunk from the cache.
    pub fn get_chunk(&self, hash: &Blake2bHash, prefix: &String) -> GetChunkFuture {
        // Start computing of chunks on the first get_chunk request.
        // Swap should ensure that this is only triggered *once* and that no race condition can occur.
        if !self.computing_enabled.swap(true, Ordering::AcqRel) {
            self.compute_chunks_for_block();
        }
        return GetChunkFuture::new(hash.clone(), prefix.clone(), self.weak_self.upgrade().unwrap());
    }

    /// Trigger computation of chunks asynchronously after blockchain events.
    fn on_blockchain_event(&self, event: &BlockchainEvent) {
        if !self.computing_enabled.load(Ordering::Acquire) {
            // Only pre-compute chunks after a chunk was requested for the first time
            return;
        }
        match event {
            BlockchainEvent::Extended(_) | BlockchainEvent::Rebranched(_, _) => {
                self.compute_chunks_for_block();
            },
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
                let mut guard = this.tasks_by_block.write();
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
            loop {
                if let Some(chunk) = this.blockchain.accounts().get_chunk(&prefix[..], AccountsChunkCache::CHUNK_SIZE_MAX, Some(&txn)) {
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
                } else {
                    break;
                }
                this.notify_tasks_for_block(&hash);
            }
            this.notify_tasks_for_block(&hash);
            // The chunks are cached, so newly created requests will not need to enter them into the list of tasks.
            this.tasks_by_block.write().remove(&hash.clone());

            let num_chunks = this.chunks_by_prefix_by_block.read().get(&hash).map(|cache| cache.len()).unwrap_or(0);
            trace!("Computing {} chunks for block {} tree took {:?}", num_chunks, hash, chunk_start.elapsed());

            // Put those blocks that are cached into a history, so that we can remove them later on.
            this.block_history_order.write().push_back(hash.clone());

            // Remove old chunks after some time.
            if this.block_history_order.read().len() > AccountsChunkCache::MAX_BLOCKS_BACKLOG {
                // Take the oldest block to remove.
                if let Some(block_hash) = this.block_history_order.write().pop_front() {
                    // First clean up the chunks.
                    this.chunks_by_prefix_by_block.write().remove(&block_hash);

                    // Then remove the tasks (if present) and notify those that there won't be an update.
                    if let Some(tasks) = this.tasks_by_block.write().remove(&block_hash) {
                        for task in tasks {
                            task.notify();
                        }
                    }
                }
            }
        });
    }

    /// Notifies tasks that something changed.
    fn notify_tasks_for_block(&self, hash: &Blake2bHash) {
        if let Some(tasks) = self.tasks_by_block.read().get(&hash) {
            for task in tasks {
                task.notify();
            }
        }
    }
}

/// Future given to the requester in order to retrieve accounts tree chunk.
pub struct GetChunkFuture {
    hash: Blake2bHash,
    prefix: String,
    chunk_cache: Arc<AccountsChunkCache>,
    running: bool
}

impl GetChunkFuture {
    pub fn new(hash: Blake2bHash, prefix: String, chunk_cache: Arc<AccountsChunkCache>) -> Self {
        return GetChunkFuture { hash, prefix, chunk_cache, running: false };
    }
}

impl Future for GetChunkFuture {
    type Item = Option<SerializedChunk>;
    type Error = ();

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        // If chunk is available, deliver it to the client.
        if let Some(chunks_by_prefix) = self.chunk_cache.chunks_by_prefix_by_block.read().get(&self.hash) {
            if let Some(chunk) = chunks_by_prefix.get(&self.prefix) {
                return Ok(Async::Ready(Some(chunk.clone())));
            }
        }
        // Otherwise check if the task is (still) filed.
        if let Some(tasks) = self.chunk_cache.tasks_by_block.write().get_mut(&self.hash) {
            // If yes, "subscribe" to updates.
            if !self.running {
                tasks.push(task::current());
                self.running = true;
            }
            return Ok(Async::NotReady);
        } else {
            // Else, the block is unknown, too far in the past or something else.
            // Return None in these cases.
            return Ok(Async::Ready(None));
        }
    }
}
