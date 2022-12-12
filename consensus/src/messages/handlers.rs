use std::{collections::HashSet, sync::Arc};

use parking_lot::RwLock;

use nimiq_block::Block;
use nimiq_blockchain::{AbstractBlockchain, Blockchain, Direction, CHUNK_SIZE};
use nimiq_blockchain_proxy::BlockchainProxy;
use nimiq_network_interface::request::Handle;

use crate::messages::*;

impl Handle<MacroChain, BlockchainProxy> for RequestMacroChain {
    fn handle(&self, blockchain: &BlockchainProxy) -> MacroChain {
        let blockchain = blockchain.read();

        // A peer has the macro chain. Check all block locator hashes in the given order and pick
        // the first hash that is found on our main chain, ignore the rest.
        let mut start_block_hash = None;
        for locator in self.locators.iter() {
            let chain_info = blockchain.get_chain_info(locator, false, None);
            if let Ok(chain_info) = chain_info {
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
                None,
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

        if let Ok(Block::Macro(block)) = blockchain.get_block(&self.hash, true, None) {
            let (batch_sets, total_history_len) = if let Ok(macro_hashes) = blockchain
                .chain_store
                .get_epoch_chunks(block.block_number(), None)
            {
                let mut total_history_len = 0usize;
                let mut previous_length = 0u32;
                let mut batch_sets = vec![];
                for macro_hash in macro_hashes {
                    let macro_block = blockchain
                        .get_block(&macro_hash, true, None)
                        .expect("Macro block must exist since it can't be pruned");
                    let tot_history_len = blockchain
                        .history_store
                        .length_at(macro_block.block_number(), None);
                    let history_len = tot_history_len - previous_length;
                    let batch_set = BatchSet {
                        macro_block: Some(macro_block.unwrap_macro()),
                        history_len,
                    };
                    batch_sets.push(batch_set);
                    total_history_len += history_len as usize;
                    previous_length += history_len / CHUNK_SIZE as u32 * CHUNK_SIZE as u32;
                }
                (batch_sets, total_history_len)
            } else {
                let history_len = blockchain
                    .history_store
                    .length_at(block.block_number(), None);
                let batch_set = BatchSet {
                    macro_block: Some(block.clone()),
                    history_len,
                };
                (vec![batch_set], history_len as usize)
            };

            let election_macro_block = if block.is_election_block() {
                Some(block)
            } else {
                None
            };

            BatchSetInfo {
                election_macro_block,
                batch_sets,
                total_history_len: total_history_len as u64,
            }
        } else {
            BatchSetInfo {
                election_macro_block: None,
                batch_sets: vec![],
                total_history_len: 0,
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

impl Handle<Option<Block>, BlockchainProxy> for RequestBlock {
    fn handle(&self, blockchain: &BlockchainProxy) -> Option<Block> {
        let blockchain = blockchain.read();
        if let Ok(block) = blockchain.get_block(&self.hash, false, None) {
            let block = match block {
                // Macro bodies are always needed
                Block::Macro(_) => match blockchain.get_block(&self.hash, true, None) {
                    Ok(block) => block,
                    Err(_) => return None,
                },
                // Micro bodies are requested based on `include_micro_bodies`
                Block::Micro(_) => {
                    if self.include_micro_bodies {
                        match blockchain.get_block(&self.hash, true, None) {
                            Ok(block) => block,
                            Err(_) => return None,
                        }
                    } else {
                        block
                    }
                }
            };
            Some(block)
        } else {
            None
        }
    }
}

impl Handle<ResponseBlocks, BlockchainProxy> for RequestMissingBlocks {
    fn handle(&self, blockchain: &BlockchainProxy) -> ResponseBlocks {
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
            let block = blockchain.get_block(&block_hash, false, None);
            if let Ok(block) = block {
                let block = match block {
                    // Macro bodies are always needed
                    Block::Macro(_) => match blockchain.get_block(&block_hash, true, None) {
                        Ok(block) => block,
                        Err(error) => {
                            debug!(
                                %error,
                                blocks_found = blocks.len(),
                                block_hash = %block_hash,
                                "ResponseBlocks - Failed to get macro block",
                            );
                            return ResponseBlocks { blocks: None };
                        }
                    },
                    // Micro bodies are requested based on `include_micro_bodies`
                    Block::Micro(_) => {
                        if self.include_micro_bodies {
                            match blockchain.get_block(&block_hash, true, None) {
                                Ok(block) => block,
                                Err(error) => {
                                    debug!(
                                        %error,
                                        include_body = self.include_micro_bodies,
                                        blocks_found = blocks.len(),
                                        block_hash = %block_hash,
                                        "ResponseBlocks - Failed to get micro block",
                                    );
                                    return ResponseBlocks { blocks: None };
                                }
                            }
                        } else {
                            // Micro bodies are not requested, so we can return the already block obtained
                            block
                        }
                    }
                };
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
                    blocks_found = blocks.len(),
                    unknown_block_hash = %block_hash,
                    "ResponseBlocks - unknown target block/predecessor",
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

impl Handle<Blake2bHash, BlockchainProxy> for RequestHead {
    fn handle(&self, blockchain: &BlockchainProxy) -> Blake2bHash {
        blockchain.read().head_hash()
    }
}
