use crate::messages::*;
use block::Block;
use blockchain::{AbstractBlockchain, Blockchain, Direction, CHUNK_SIZE};
use network_interface::message::ResponseMessage;
use primitives::policy;
use std::sync::Arc;

/// This trait defines the behaviour when receiving a message and how to generate the response.
pub trait Handle<Response> {
    fn handle(&self, blockchain: &Arc<Blockchain>) -> Response;
}

impl Handle<BlockHashes> for RequestBlockHashes {
    fn handle(&self, blockchain: &Arc<Blockchain>) -> BlockHashes {
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
    fn handle(&self, blockchain: &Arc<Blockchain>) -> BatchSetInfo {
        if let Some(Block::Macro(block)) = blockchain.get_block(&self.hash, true, None) {
            // Leaf indices are 0 based thus the + 1
            let history_len = blockchain
                .history_store
                // TODO Refactor get_last_leaf_index_of_block
                .get_last_leaf_index_of_block(block.header.block_number, None)
                + 1;
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
    fn handle(&self, blockchain: &Arc<Blockchain>) -> HistoryChunk {
        let chunk = blockchain.history_store.prove_chunk(
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
    fn handle(&self, blockchain: &Arc<Blockchain>) -> ResponseBlock {
        let block = blockchain.get_block(&self.hash, true, None);
        ResponseBlock {
            block,
            request_identifier: self.get_request_identifier(),
        }
    }
}

impl Handle<ResponseBlocks> for RequestMissingBlocks {
    fn handle(&self, blockchain: &Arc<Blockchain>) -> ResponseBlocks {
        // Behaviour of our missing blocks request:
        // 1. Receives `target_block_hash: Blake2bHash, locators: Vec<Blake2bHash>`
        // 2. Return all blocks in between most recent locator on our main chain and target block hash
        // 3. This should also work across 1 or 2 batches with an upper bound
        // For now, we just ignore the case if we receive a block announcement which is more than 1-2 batches away from our current block.
        let target_block_opt = blockchain.get_block(&self.target_hash, false, None);
        if target_block_opt.is_none() {
            return ResponseBlocks {
                blocks: None,
                request_identifier: self.get_request_identifier(),
            };
        }

        let target_block = target_block_opt.unwrap();

        // A peer has requested blocks. Check all requested block locator hashes
        // in the given order and pick the first hash that is found on our main
        // chain, ignore the rest. If none of the requested hashes is found,
        // pick the last macro block hash. Send the main chain starting from the
        // picked hash back to the peer.
        let mut start_block = None;
        for locator in self.locators.iter() {
            if let Some(block) = blockchain.chain_store.get_block(locator, false, None) {
                // We found a block, ignore remaining block locator hashes.
                start_block = Some(block);
                break;
            }
        }

        // if no start_block can be found, assume the last macro block before target_block
        let start_block = if start_block.is_none() {
            if let Some(block) = blockchain.get_block_at(
                policy::macro_block_before(target_block.block_number()),
                false,
                None,
            ) {
                block
            } else {
                return ResponseBlocks {
                    blocks: None,
                    request_identifier: self.get_request_identifier(),
                };
            }
        } else {
            start_block.unwrap()
        };

        // Check that the distance is sensible.
        let num_blocks = target_block.block_number() - start_block.block_number();
        if num_blocks > policy::BATCH_LENGTH * 2 {
            debug!("Received missing block request across more than 2 batches.");
            return ResponseBlocks {
                blocks: None,
                request_identifier: self.get_request_identifier(),
            };
        }

        // Collect the blocks starting right after the identified block on the main chain
        // up to our target hash.
        let blocks =
            blockchain.get_blocks(&start_block.hash(), num_blocks, true, Direction::Forward);

        ResponseBlocks {
            blocks: Some(blocks),
            request_identifier: self.get_request_identifier(),
        }
    }
}

impl Handle<HeadResponse> for RequestHead {
    fn handle(&self, blockchain: &Arc<Blockchain>) -> HeadResponse {
        let hash = blockchain.head_hash();
        HeadResponse {
            hash,
            request_identifier: self.get_request_identifier(),
        }
    }
}
