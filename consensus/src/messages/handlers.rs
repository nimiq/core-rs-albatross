use std::{collections::HashSet, sync::Arc};

use parking_lot::RwLock;

use nimiq_block::Block;
use nimiq_blockchain::{AbstractBlockchain, Blockchain, Direction, CHUNK_SIZE};

use crate::messages::*;

/// This trait defines the behaviour when receiving a message and how to generate the response.
pub trait Handle<Response, T> {
    fn handle(&self, blockchain: &T) -> Response;
}

impl Handle<MacroChain, Arc<RwLock<Blockchain>>> for RequestMacroChain {
    fn handle(&self, blockchain: &Arc<RwLock<Blockchain>>) -> MacroChain {
        let blockchain = blockchain.read();

        // A peer has the macro chain. Check all block locator hashes in the given order and pick
        // the first hash that is found on our main chain, ignore the rest.
        let mut start_block_hash = None;
        for locator in self.locators.iter() {
            let chain_info = blockchain.chain_store.get_chain_info(locator, false, None);
            if let Some(chain_info) = chain_info {
                if chain_info.on_main_chain {
                    // We found a block, ignore remaining block locator hashes.
                    trace!("Start block found: {:?}", &locator);
                    start_block_hash = Some(locator.clone());
                    break;
                }
            }
        }
        if start_block_hash.is_none() {
            return MacroChain {
                epochs: None,
                checkpoint: None,
            };
        }
        let start_block_hash = start_block_hash.unwrap();

        // Get up to `self.max_blocks` macro blocks from our chain starting at `start_block_hash`.
        // TODO We don't need the actual macro block headers here, the hash of each block would suffice.
        let election_blocks = blockchain
            .get_macro_blocks(
                &start_block_hash,
                self.max_epochs as u32,
                false,
                Direction::Forward,
                true,
            )
            .unwrap(); // We made sure that start_block_hash is on our chain.
        let epochs: Vec<_> = election_blocks.iter().map(|block| block.hash()).collect();

        // Add latest checkpoint block if all of the following conditions are met:
        // * the requester has caught up, i.e. it already knows the last epoch (epochs.is_empty())
        // * the latest macro block is a checkpoint block.
        // * the latest macro block is not the locator given by the requester.
        let checkpoint_block = blockchain.macro_head();
        let checkpoint_hash = blockchain.macro_head_hash();
        let checkpoint = if epochs.is_empty()
            && !checkpoint_block.is_election_block()
            && checkpoint_hash != start_block_hash
        {
            Some(Checkpoint {
                block_number: checkpoint_block.block_number(),
                hash: checkpoint_hash,
            })
        } else {
            None
        };

        MacroChain {
            epochs: Some(epochs),
            checkpoint,
        }
    }
}

impl Handle<BatchSetInfo, Arc<RwLock<Blockchain>>> for RequestBatchSet {
    fn handle(&self, blockchain: &Arc<RwLock<Blockchain>>) -> BatchSetInfo {
        let blockchain = blockchain.read();

        if let Some(Block::Macro(block)) = blockchain.get_block(&self.hash, true, None) {
            let history_len = blockchain
                .history_store
                .length_at(block.header.block_number, None);

            BatchSetInfo {
                block: Some(block),
                history_len,
            }
        } else {
            BatchSetInfo {
                block: None,
                history_len: 0,
            }
        }
    }
}

impl Handle<HistoryChunk, Arc<RwLock<Blockchain>>> for RequestHistoryChunk {
    fn handle(&self, blockchain: &Arc<RwLock<Blockchain>>) -> HistoryChunk {
        let chunk = blockchain.read().history_store.prove_chunk(
            self.epoch_number,
            self.block_number,
            CHUNK_SIZE,
            self.chunk_index as usize,
            None,
        );
        HistoryChunk { chunk }
    }
}

impl Handle<Option<Block>, Arc<RwLock<Blockchain>>> for RequestBlock {
    fn handle(&self, blockchain: &Arc<RwLock<Blockchain>>) -> Option<Block> {
        blockchain.read().get_block(&self.hash, true, None)
    }
}

impl Handle<ResponseBlocks, Arc<RwLock<Blockchain>>> for RequestMissingBlocks {
    fn handle(&self, blockchain: &Arc<RwLock<Blockchain>>) -> ResponseBlocks {
        let blockchain = blockchain.read();

        // TODO We might want to do a sanity check on the locator hashes and reject the request if
        //  they they don't match up with the given target hash.

        // Build a HashSet from the given locator hashes.
        let locators = HashSet::<Blake2bHash>::from_iter(self.locators.iter().cloned());

        // Walk the chain backwards from the target block until we find one of the locators or
        // encounter a macro block. Return all blocks between the locator block (exclusive) and the
        // target block (inclusive). If we stopped at a macro block instead of a locator, the macro
        // block is included in the result.
        let mut blocks = Vec::new();
        let mut block_hash = self.target_hash.clone();
        while !locators.contains(&block_hash) {
            let block = blockchain.get_block(&block_hash, true, None);
            if let Some(block) = block {
                let is_macro = block.is_macro();

                block_hash = block.parent_hash().clone();
                blocks.push(block);

                if is_macro {
                    break;
                }
            } else {
                // This can only happen if the target hash is unknown or after the chain was pruned.
                // TODO Return the blocks we found instead of failing here?
                debug!(
                    "ResponseBlocks - unknown target block/predecessor {} ({} blocks found)",
                    block_hash,
                    blocks.len(),
                );
                return ResponseBlocks { blocks: None };
            }
        }

        // Blocks are returned in ascending (forward) order.
        blocks.reverse();

        ResponseBlocks {
            blocks: Some(blocks),
        }
    }
}

impl Handle<Blake2bHash, Arc<RwLock<Blockchain>>> for RequestHead {
    fn handle(&self, blockchain: &Arc<RwLock<Blockchain>>) -> Blake2bHash {
        blockchain.read().head_hash()
    }
}
