use std::collections::HashMap;
use std::sync::{Arc, Weak};

use parking_lot::RwLock;
use futures::future::Future;
use futures::Poll;
use futures::prelude::*;
use futures::task::Task;

use accounts::tree::AccountsTreeChunk;
use blockchain::{Blockchain, BlockchainEvent};
use database::Environment;
use database::ReadTransaction;
use hash::Blake2bHash;
use utils::mutable_once::MutableOnce;

use std::thread;
use hash::Hash;
use std::error::Error;
use futures::task;
use std::collections::LinkedList;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;

pub struct AccountsChunkCache {
    blockchain: Arc<Blockchain<'static>>,
    env: &'static Environment,
    computing_enabled: AtomicBool,
    chunks_by_prefix_by_block: RwLock<HashMap<Blake2bHash, HashMap<String, AccountsTreeChunk>>>,
    tasks_by_block: RwLock<HashMap<Blake2bHash, Vec<Task>>>,
    block_history_order: RwLock<LinkedList<Blake2bHash>>,
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
            chunks_by_prefix_by_block: RwLock::new(HashMap::new()),
            tasks_by_block: RwLock::new(HashMap::new()),
            block_history_order: RwLock::new(LinkedList::new()),
            weak_self: MutableOnce::new(Weak::new()),
        };
        let cache_arc = Arc::new(cache);
        unsafe { cache_arc.weak_self.replace(Arc::downgrade(&cache_arc)) };

        let cache_arc2 = Arc::clone(&cache_arc);
        cache_arc.blockchain.notifier.write().register(move |event: &BlockchainEvent| cache_arc2.on_blockchain_event(event));

        cache_arc
    }

    pub fn get_chunk(&self, hash: &Blake2bHash, prefix: &String) -> GetChunkFuture {
        if !self.computing_enabled.load(Ordering::Relaxed) {
            self.computing_enabled.store(true, Ordering::Relaxed);
            if &self.blockchain.head_hash() == hash {
                self.compute_chunks_for_block(hash.clone());
            }
        }
        return GetChunkFuture::new(hash.clone(), prefix.clone(), self.weak_self.upgrade().unwrap());
    }

    fn on_blockchain_event(&self, event: &BlockchainEvent) {
        if !self.computing_enabled.load(Ordering::Relaxed) {
            // Only pre-compute chunks after a chunk was requested for the first time
            return;
        }
        match event {
            BlockchainEvent::Extended(_, ref block) => {
                self.compute_chunks_for_block(block.header.hash());
            },
            BlockchainEvent::Rebranched(_, ref adopted_blocks) => {
                for (_hash, block) in adopted_blocks {
                    self.compute_chunks_for_block(block.header.hash());
                }
            }
        }
    }

    fn compute_chunks_for_block(&self, hash: Blake2bHash) {
        // Requests for accounts tree chunks of `block` are accepted now
        self.tasks_by_block.write().insert(hash.clone(), Vec::new());

        let this = upgrade_weak!(self.weak_self);
        thread::spawn(move || {
            let tx = ReadTransaction::new(&this.env);
            this.chunks_by_prefix_by_block.write().insert(hash.clone(), HashMap::new());
            let mut prefix = "".to_string();
            loop {
                if let Some(chunk) = this.blockchain.accounts().get_chunk(&prefix[..], AccountsChunkCache::CHUNK_SIZE_MAX, Some(&tx)) {
                    if let Some(chunks_by_prefix) = this.chunks_by_prefix_by_block.write().get_mut(&hash) {
                        let last_terminal_string_opt = chunk.last_terminal_string();
                        let chunk_len = chunk.len();
                        chunks_by_prefix.insert(prefix.clone(), chunk);
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
            this.block_history_order.write().push_back(hash.clone());

            // Remove old chunks
            if this.block_history_order.read().len() > AccountsChunkCache::MAX_BLOCKS_BACKLOG {
                if let Some(block_hash) = this.block_history_order.write().pop_front() {
                    this.chunks_by_prefix_by_block.write().remove(&block_hash);
                    if let Some(tasks) = this.tasks_by_block.read().get(&block_hash) {
                        for task in tasks {
                            task.notify();
                        }
                    }
                    this.tasks_by_block.write().remove(&block_hash);
                }
            }

            this.tasks_by_block.write().remove(&hash.clone());
        });
    }

    fn notify_tasks_for_block(&self, hash: &Blake2bHash) {
        if let Some(tasks) = self.tasks_by_block.read().get(&hash) {
            for task in tasks {
                task.notify();
            }
        }
    }
}

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
    type Item = Option<AccountsTreeChunk>;
    type Error = Box<Error>;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        if let Some(chunks_by_prefix) = self.chunk_cache.chunks_by_prefix_by_block.read().get(&self.hash) {
            if let Some(chunk) = chunks_by_prefix.get(&self.prefix) {
                return Ok(Async::Ready(Some(chunk.clone())));
            }
        }
        if let Some(tasks) = self.chunk_cache.tasks_by_block.write().get_mut(&self.hash) {
            if !self.running {
                tasks.push(task::current());
                self.running = true;
            }
            return Ok(Async::NotReady);
        } else {
            return Ok(Async::Ready(None));
        }
    }
}
