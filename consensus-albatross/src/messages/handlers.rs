use crate::messages::{
    BatchSetInfo, BlockHashType, BlockHashes, HistoryChunk, RequestBatchSet, RequestBlock,
    RequestBlockHashes, RequestBlockHashesFilter, RequestHistoryChunk, RequestMissingBlocks,
    ResponseBlock, ResponseBlocks,
};
use block_albatross::Block;
use blockchain_albatross::{history_store::CHUNK_SIZE, Blockchain, Direction};
use network_interface::message::ResponseMessage;
use nimiq_genesis::NetworkInfo;
use primitives::policy;
use std::sync::Arc;

/// This trait defines the behaviour when receiving a message and how to generate the response.
pub trait Handle<Response> {
    fn handle(&self, blockchain: &Arc<Blockchain>) -> Option<Response>;
}

impl Handle<BlockHashes> for RequestBlockHashes {
    fn handle(&self, blockchain: &Arc<Blockchain>) -> Option<BlockHashes> {
        // A peer has requested blocks. Check all requested block locator hashes
        // in the given order and pick the first hash that is found on our main
        // chain, ignore the rest. If none of the requested hashes is found,
        // pick the genesis block hash. Send the main chain starting from the
        // picked hash back to the peer.
        let network_info = NetworkInfo::from_network_id(blockchain.network_id);
        let mut start_block_hash = network_info.genesis_hash().clone();
        for locator in self.locators.iter() {
            if blockchain
                .chain_store
                .get_block(locator, false, None)
                .is_some()
            {
                // We found a block, ignore remaining block locator hashes.
                start_block_hash = locator.clone();
                break;
            }
        }

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
            if !checkpoint_block.is_election_block() {
                hashes.push((BlockHashType::Checkpoint, checkpoint_block.hash()));
            }
        }

        Some(BlockHashes {
            hashes,
            request_identifier: self.get_request_identifier(),
        })
    }
}

impl Handle<BatchSetInfo> for RequestBatchSet {
    fn handle(&self, blockchain: &Arc<Blockchain>) -> Option<BatchSetInfo> {
        if let Some(Block::Macro(block)) = blockchain.get_block(&self.hash, true) {
            let epoch = policy::epoch_at(block.header.block_number);
            let history_len = blockchain.get_num_extended_transactions(epoch, None);
            let response = BatchSetInfo {
                block,
                history_len: history_len as u32,
                request_identifier: self.get_request_identifier(),
            };

            Some(response)
        } else {
            None
        }
    }
}

impl Handle<HistoryChunk> for RequestHistoryChunk {
    fn handle(&self, blockchain: &Arc<Blockchain>) -> Option<HistoryChunk> {
        let chunk = blockchain.get_chunk(
            self.epoch_number,
            CHUNK_SIZE,
            self.chunk_index as usize,
            None,
        );
        let response = HistoryChunk {
            chunk,
            request_identifier: self.get_request_identifier(),
        };
        Some(response)
    }
}

impl Handle<ResponseBlock> for RequestBlock {
    fn handle(&self, blockchain: &Arc<Blockchain>) -> Option<ResponseBlock> {
        let block = blockchain.get_block(&self.hash, true);
        let response = ResponseBlock {
            block,
            request_identifier: self.get_request_identifier(),
        };
        Some(response)
    }
}

impl Handle<ResponseBlocks> for RequestMissingBlocks {
    fn handle(&self, blockchain: &Arc<Blockchain>) -> Option<ResponseBlocks> {
        // Behaviour of our missing blocks request:
        // 1. Receives `target_block_hash: Blake2bHash, locators: Vec<Blake2bHash>`
        // 2. Return all blocks in between most recent locator on our main chain and target block hash
        // 3. This should also work across 1 or 2 batches with an upper bound
        // For now, we just ignore the case if we receive a block announcement which is more than 1-2 batches away from our current block.
        let target_block = blockchain.get_block(&self.target_hash, false)?;

        // A peer has requested blocks. Check all requested block locator hashes
        // in the given order and pick the first hash that is found on our main
        // chain, ignore the rest. If none of the requested hashes is found,
        // pick the genesis block hash. Send the main chain starting from the
        // picked hash back to the peer.
        let mut start_block = None;
        for locator in self.locators.iter() {
            if let Some(block) = blockchain.chain_store.get_block(locator, false, None) {
                // We found a block, ignore remaining block locator hashes.
                start_block = Some(block);
                break;
            }
        }

        // Check that the distance is sensible.
        let start_block = start_block?;
        let num_blocks = target_block.block_number() - start_block.block_number();
        if num_blocks > policy::BATCH_LENGTH * 2 {
            debug!("Received missing block request across more than 2 batches.");
            return None;
        }

        // Collect the blocks starting right after the identified block on the main chain
        // up to our target hash.
        let blocks =
            blockchain.get_blocks(&start_block.hash(), num_blocks, true, Direction::Forward);

        Some(ResponseBlocks {
            blocks,
            request_identifier: self.get_request_identifier(),
        })
    }
}
