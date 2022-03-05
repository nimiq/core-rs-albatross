use std::collections::HashSet;
use std::sync::Arc;

use parking_lot::RwLock;

use nimiq_block::Block;
use nimiq_blockchain::{AbstractBlockchain, Blockchain, Direction, CHUNK_SIZE};
use nimiq_network_interface::message::ResponseMessage;

use crate::messages::*;

/// This trait defines the behaviour when receiving a message and how to generate the response.
pub trait Handle<Response> {
    fn handle(&self, blockchain: &Arc<RwLock<Blockchain>>) -> Response;
}

impl Handle<BlockHashes> for RequestBlockHashes {
    fn handle(&self, blockchain: &Arc<RwLock<Blockchain>>) -> BlockHashes {
        let blockchain = blockchain.read();
        // A peer has requested blocks. Check all requested block locator hashes
        // in the given order and pick the first hash that is found on our main
        // chain, ignore the rest.
        let mut start_block_hash_opt = None;
        for locator in self.locators.iter() {
            if blockchain
                .chain_store
                .get_block(locator, false, None)
                .is_some()
            {
                // We found a block, ignore remaining block locator hashes.
                trace!("Start block found: {:?}", &locator);
                start_block_hash_opt = Some(locator.clone());
                break;
            }
        }
        if start_block_hash_opt.is_none() {
            return BlockHashes {
                hashes: None,
                request_identifier: self.get_request_identifier(),
            };
        }
        let start_block_hash = start_block_hash_opt.unwrap();

        // Collect up to GETBLOCKS_VECTORS_MAX inventory vectors for the blocks starting right
        // after the identified block on the main chain.
        let blocks = match self.filter {
            RequestBlockHashesFilter::ElectionOnly
            | RequestBlockHashesFilter::ElectionAndLatestCheckpoint => blockchain
                .get_macro_blocks(
                    &start_block_hash,
                    self.max_blocks as u32,
                    false,
                    Direction::Forward,
                    true,
                )
                .unwrap(), // We made sure that start_block_hash is on our chain.
            RequestBlockHashesFilter::All => blockchain.get_blocks(
                &start_block_hash,
                self.max_blocks as u32,
                false,
                Direction::Forward,
            ),
        };

        let mut hashes: Vec<_> = blocks
            .iter()
            .map(|block| (BlockHashType::from(block), block.hash()))
            .collect();

        // Add latest checkpoint block if requested.
        if self.filter == RequestBlockHashesFilter::ElectionAndLatestCheckpoint
            && hashes.len() < self.max_blocks as usize
        {
            let checkpoint_block = blockchain.macro_head();
            // Only include the latest checkpoint block if it is not the locator given by the requester
            if !checkpoint_block.is_election_block() && checkpoint_block.hash() != start_block_hash
            {
                hashes.push((BlockHashType::Checkpoint, checkpoint_block.hash()));
            }
        }

        BlockHashes {
            hashes: Some(hashes),
            request_identifier: self.get_request_identifier(),
        }
    }
}

impl Handle<BatchSetInfo> for RequestBatchSet {
    fn handle(&self, blockchain: &Arc<RwLock<Blockchain>>) -> BatchSetInfo {
        let blockchain = blockchain.read();
        if let Some(Block::Macro(block)) = blockchain.get_block(&self.hash, true, None) {
            let history_len = blockchain
                .history_store
                .length_at(block.header.block_number, None);

            BatchSetInfo {
                block: Some(block),
                history_len,
                request_identifier: self.get_request_identifier(),
            }
        } else {
            BatchSetInfo {
                block: None,
                history_len: 0,
                request_identifier: self.get_request_identifier(),
            }
        }
    }
}

impl Handle<HistoryChunk> for RequestHistoryChunk {
    fn handle(&self, blockchain: &Arc<RwLock<Blockchain>>) -> HistoryChunk {
        let chunk = blockchain.read().history_store.prove_chunk(
            self.epoch_number,
            self.block_number,
            CHUNK_SIZE,
            self.chunk_index as usize,
            None,
        );
        HistoryChunk {
            chunk,
            request_identifier: self.get_request_identifier(),
        }
    }
}

impl Handle<ResponseBlock> for RequestBlock {
    fn handle(&self, blockchain: &Arc<RwLock<Blockchain>>) -> ResponseBlock {
        let block = blockchain.read().get_block(&self.hash, true, None);
        ResponseBlock {
            block,
            request_identifier: self.get_request_identifier(),
        }
    }
}

impl Handle<ResponseBlocks> for RequestMissingBlocks {
    fn handle(&self, blockchain: &Arc<RwLock<Blockchain>>) -> ResponseBlocks {
        let blockchain = blockchain.read();

        // TODO We might want to do a sanity check on the locator hashes and reject the request if
        //  they they don't match up with the given target hash.

        // Build a HashSet from the given locator hashes.
        let locators = HashSet::<Blake3Hash>::from_iter(self.locators.iter().cloned());

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
                    "ResponseBlocks [{}] - unknown target block/predecessor {} ({} blocks found)",
                    self.request_identifier,
                    block_hash,
                    blocks.len(),
                );
                return ResponseBlocks {
                    blocks: None,
                    request_identifier: self.get_request_identifier(),
                };
            }
        }

        // Blocks are returned in ascending (forward) order.
        blocks.reverse();

        ResponseBlocks {
            blocks: Some(blocks),
            request_identifier: self.get_request_identifier(),
        }
    }
}

impl Handle<HeadResponse> for RequestHead {
    fn handle(&self, blockchain: &Arc<RwLock<Blockchain>>) -> HeadResponse {
        let hash = blockchain.read().head_hash();
        HeadResponse {
            hash,
            request_identifier: self.get_request_identifier(),
        }
    }
}
