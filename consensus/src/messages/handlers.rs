use std::cmp;
#[cfg(feature = "full")]
use std::sync::Arc;

use nimiq_block::Block;
#[cfg(feature = "full")]
use nimiq_block::BlockInclusionProof;
#[cfg(feature = "full")]
use nimiq_blockchain::{Blockchain, CHUNK_SIZE};
use nimiq_blockchain_interface::{AbstractBlockchain, BlockchainError, Direction};
use nimiq_blockchain_proxy::BlockchainProxy;
use nimiq_network_interface::{network::Network, request::Handle};
use nimiq_primitives::policy::Policy;
#[cfg(feature = "full")]
use parking_lot::RwLock;

use crate::messages::*;
#[cfg(feature = "full")]
use crate::sync::live::{
    diff_queue::{RequestTrieDiff, ResponseTrieDiff},
    state_queue::{Chunk, RequestChunk, ResponseChunk},
};

impl<N: Network> Handle<N, BlockchainProxy> for RequestMacroChain {
    fn handle(&self, _peer_id: N::PeerId, blockchain: &BlockchainProxy) -> MacroChain {
        let blockchain = blockchain.read();

        // A peer has the macro chain. Check all block locator hashes in the given order and pick
        // the first hash that is found on our main chain, ignore the rest.
        let mut start_block_hash = None;
        for locator in self.locators.iter() {
            let chain_info = blockchain.get_chain_info(locator, false);
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
            )
            .unwrap(); // We made sure that start_block_hash is on our chain.
        let epochs: Vec<_> = election_blocks.iter().map(|block| block.hash()).collect();

        // Add latest checkpoint block if all of the following conditions are met:
        // * the latest macro block is a checkpoint block.
        // * the latest macro block is not the locator given by the requester.
        // * the requester has caught up, i.e. it already knows the most recent epoch (epochs.is_empty())
        //   or we are returning the most recent epoch as part of this response.
        let checkpoint_block = blockchain.macro_head();
        let checkpoint_hash = blockchain.macro_head_hash();
        let caught_up = epochs.is_empty()
            || *epochs.last().unwrap() == checkpoint_block.header.parent_election_hash;
        let checkpoint = if !checkpoint_block.is_election_block()
            && checkpoint_hash != start_block_hash
            && caught_up
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

#[cfg(feature = "full")]
impl<N: Network> Handle<N, Arc<RwLock<Blockchain>>> for RequestBatchSet {
    fn handle(&self, _peer_id: N::PeerId, blockchain: &Arc<RwLock<Blockchain>>) -> BatchSetInfo {
        let blockchain = blockchain.read();

        let block = match blockchain.get_block(&self.hash, true, None) {
            Ok(Block::Macro(block)) => block,
            _ => return BatchSetInfo::default(),
        };

        let batch_sets = if let Ok(macro_hashes) = blockchain
            .chain_store
            .get_epoch_chunks(block.block_number(), None)
        {
            let mut batch_sets = vec![];
            for macro_hash in macro_hashes {
                let macro_block = blockchain
                    .get_block(&macro_hash, true, None)
                    .expect("Macro block must exist since it can't be pruned");

                let history_len = blockchain
                    .history_store
                    .prove_num_leaves(macro_block.block_number(), None)
                    .expect("Failed to prove history size");

                let batch_set = BatchSet {
                    macro_block: macro_block.unwrap_macro(),
                    history_len,
                };
                batch_sets.push(batch_set);
            }
            batch_sets
        } else {
            let history_len = blockchain
                .history_store
                .prove_num_leaves(block.block_number(), None)
                .expect("Failed to prove history size");

            let batch_set = BatchSet {
                macro_block: block.clone(),
                history_len,
            };
            vec![batch_set]
        };

        // FIXME Don't send the same macro block twice.
        let election_macro_block = if block.is_election_block() {
            Some(block)
        } else {
            None
        };

        BatchSetInfo {
            election_macro_block,
            batch_sets,
        }
    }
}

#[cfg(feature = "full")]
impl<N: Network> Handle<N, Arc<RwLock<Blockchain>>> for RequestHistoryChunk {
    fn handle(&self, _peer_id: N::PeerId, blockchain: &Arc<RwLock<Blockchain>>) -> HistoryChunk {
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

impl<N: Network> Handle<N, BlockchainProxy> for RequestBlock {
    fn handle(&self, _peer_id: N::PeerId, blockchain: &BlockchainProxy) -> Option<Block> {
        let blockchain = blockchain.read();
        if let Ok(block) = blockchain.get_block(&self.hash, false) {
            let block = match block {
                // Macro bodies are always needed
                Block::Macro(_) => match blockchain.get_block(&self.hash, true) {
                    Ok(block) => block,
                    Err(_) => return None,
                },
                // Micro bodies are requested based on `include_micro_bodies`
                Block::Micro(_) => {
                    if self.include_micro_bodies {
                        match blockchain.get_block(&self.hash, true) {
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

impl<N: Network> Handle<N, BlockchainProxy> for RequestMissingBlocks {
    fn handle(&self, _peer_id: N::PeerId, blockchain: &BlockchainProxy) -> ResponseBlocks {
        let blockchain = blockchain.read();

        // Check that we know the target hash and that it is located on our main chain.
        let target_block = match blockchain.get_chain_info(&self.target_hash, false) {
            Ok(target_block) => {
                if !target_block.on_main_chain {
                    debug!(
                        target_hash = %self.target_hash,
                        "ResponseBlocks - target block not on main chain",
                    );
                    return ResponseBlocks { blocks: None };
                }
                target_block
            }
            Err(error) => {
                debug!(
                    %error,
                    target_hash = %self.target_hash,
                    "ResponseBlocks - target hash not found",
                );
                return ResponseBlocks { blocks: None };
            }
        };

        // Find the first locator that is on our main chain.
        // The locators are ordered from newest to oldest block.
        let mut start_hash = None;
        let mut start_block_number = None;
        for locator in self.locators.iter() {
            if let Ok(chain_info) = blockchain.get_chain_info(locator, false) {
                if chain_info.on_main_chain {
                    start_hash = Some(locator.clone());
                    start_block_number = Some(chain_info.head.block_number());
                    break;
                }
            }
        }

        // If there is no match, reject the request.
        if start_hash.is_none() {
            debug!("ResponseBlocks - unknown locators",);
            return ResponseBlocks { blocks: None };
        }
        let start_block_number = start_block_number.unwrap();
        let start_hash = start_hash.unwrap();

        // Get at most one batch of blocks from there.
        let next_macro_block = Policy::macro_block_after(start_block_number);
        let num_blocks = cmp::min(
            next_macro_block - start_block_number,
            target_block
                .head
                .block_number()
                .saturating_sub(start_block_number),
        );

        // If the number of blocks to return is 0, we return early.
        if num_blocks == 0 {
            return ResponseBlocks {
                blocks: Some(vec![]),
            };
        }

        // Request `num_blocks - 1` micro blocks first and add the following macro block separately.
        // We do this because we always include the body for macro blocks.
        let mut blocks = match blockchain.get_blocks(
            &start_hash,
            num_blocks - 1,
            self.include_micro_bodies,
            Direction::Forward,
        ) {
            Ok(blocks) => blocks,
            Err(error) => {
                debug!(
                    %error,
                    start_hash = %start_hash,
                    "ResponseBlocks - Failed to get blocks",
                );
                return ResponseBlocks { blocks: None };
            }
        };

        if let Ok(block) = blockchain.get_block_at(
            start_block_number + num_blocks,
            self.include_micro_bodies || Policy::is_macro_block_at(start_block_number + num_blocks),
        ) {
            blocks.push(block);
        }

        ResponseBlocks {
            blocks: Some(blocks),
        }
    }
}

impl<N: Network> Handle<N, BlockchainProxy> for RequestHead {
    fn handle(&self, _peer_id: N::PeerId, blockchain: &BlockchainProxy) -> Blake2bHash {
        blockchain.read().head_hash()
    }
}

#[cfg(feature = "full")]
impl<N: Network> Handle<N, Arc<RwLock<Blockchain>>> for RequestChunk {
    fn handle(&self, _peer_id: N::PeerId, blockchain: &Arc<RwLock<Blockchain>>) -> ResponseChunk {
        let blockchain_rg = blockchain.read();

        // Check if our state is complete.
        let txn = blockchain_rg.read_transaction();
        if !blockchain_rg.state.accounts.is_complete(Some(&txn)) {
            return ResponseChunk::IncompleteState;
        }

        let chunk = blockchain_rg.state.accounts.get_chunk(
            self.start_key.clone(),
            cmp::min(self.limit, Policy::state_chunks_max_size()) as usize,
            Some(&txn),
        );
        ResponseChunk::Chunk(Chunk {
            block_number: blockchain_rg.block_number(),
            block_hash: blockchain_rg.head_hash(),
            chunk,
        })
    }
}

#[cfg(feature = "full")]
impl<N: Network> Handle<N, Arc<RwLock<Blockchain>>> for RequestTrieDiff {
    fn handle(&self, _peer_id: N::PeerId, context: &Arc<RwLock<Blockchain>>) -> ResponseTrieDiff {
        // TODO return the requested range only
        let blockchain = context.read();
        let txn = blockchain.read_transaction();
        match blockchain
            .chain_store
            .get_accounts_diff(&self.block_hash, Some(&txn))
        {
            Ok(diff) => ResponseTrieDiff::PartialDiff(diff),
            Err(BlockchainError::BlockNotFound) => ResponseTrieDiff::UnknownBlockHash,
            Err(BlockchainError::AccountsDiffNotFound) => ResponseTrieDiff::IncompleteState,
            Err(e) => {
                error!("unexpected error while querying accounts diff: {}", e);
                ResponseTrieDiff::IncompleteState
            }
        }
    }
}

#[cfg(feature = "full")]
fn prove_txns_with_block_number(
    blockchain: &Arc<RwLock<Blockchain>>,
    transactions: &[Blake2bHash],
    block_number: u32,
) -> ResponseTransactionsProof {
    let blockchain = blockchain.read();
    let hashes: Vec<&Blake2bHash> = transactions.iter().collect();

    // There are three possible scenarios for creating a transaction inclusion proof:
    // A- The block number is located in a finalized epoch:
    //    We use the epoch's finalization block (election block)
    // B- The block number is located in an incomplete epoch with an already finalized checkpoint block
    //    We use the batch's checkpoint block, with the current transaction count to construct the proof
    // C- The block number is located in the current batch:
    //    We use the current transaction count to construct the proof

    let mut verifier_state = None;
    let election_head = blockchain.election_head().block_number();
    let macro_head = blockchain.macro_head().block_number();
    let current_head = blockchain.head().block_number();

    // We cannot prove transactions from the future
    if block_number > current_head {
        log::info!(
            current_head,
            requested_block_number = block_number,
            "Requested txn proof from the future",
        );
        return ResponseTransactionsProof {
            proof: None,
            block: None,
        };
    }

    let proving_block_number = if Policy::is_election_block_at(block_number) {
        // If we were provided the block number of an election block it has to be already finalized
        log::info!(
            election_block_number = block_number,
            len = hashes.len(),
            "Requested txn proof from finalized epoch",
        );
        block_number
    } else if Policy::is_macro_block_at(block_number) {
        log::info!(
            checkpoint_block_number = block_number,
            len = hashes.len(),
            "Requested txn proof from finalized checkpoint block",
        );
        // If we were provided a block number corresponding to a checkpoint block, it needs to correspond to the current epoch
        // Otherwise, the requester should use the latest epoch number.
        if block_number < election_head {
            log::info!(
                block_number = block_number,
                "Requested txn proof that corresponds to a finalized epoch, should use the election block instead",
            );
            return ResponseTransactionsProof {
                proof: None,
                block: None,
            };
        }
        block_number
    } else {
        log::info!(
            block_number = block_number,
            len = hashes.len(),
            "Requested txn proof from current batch",
        );
        // If we were provided a block number corresponding to a micro block,
        // it needs to correspond to at least the previous batch (we allow the
        // previous batch also, to not fail when the client is a few blocks behind
        // and we are already at or over a macro block).
        // If the requested block is older than the previous batch, the requester
        // should use the latest checkpoint block number instead.
        if block_number < macro_head - Policy::blocks_per_batch() {
            log::info!(
                block_number = block_number,
                "Requested txn proof from finalized batch, should use a checkpoint block instead",
            );
            return ResponseTransactionsProof {
                proof: None,
                block: None,
            };
        }
        block_number
    };

    let block = blockchain
        .chain_store
        .get_block_at(proving_block_number, false, None)
        .ok();

    if let Some(ref block) = block {
        // We have some extra work in the current epoch, if the block we are proving is not our current head
        if block.block_number() > election_head && block.block_number() < current_head {
            let chain_info = blockchain.get_chain_info(&block.hash(), false, None);
            let history_tree_len = chain_info.unwrap().history_tree_len;
            verifier_state = Some(history_tree_len as usize);
        }
    } else {
        log::info!("Could not find the desired block to create the txn proof");
        return ResponseTransactionsProof {
            proof: None,
            block: None,
        };
    }

    let proof = blockchain.history_store.prove(
        Policy::epoch_at(proving_block_number),
        hashes,
        verifier_state,
        None,
    );

    // If we couldn't obtain a proof we return None
    if proof.is_none() {
        log::info!("Could not generate the txn inclusion proof");
        return ResponseTransactionsProof {
            proof: None,
            block: None,
        };
    }

    ResponseTransactionsProof { proof, block }
}

#[cfg(feature = "full")]
fn prove_transaction(
    blockchain: &Arc<RwLock<Blockchain>>,
    transaction: &Blake2bHash,
) -> ResponseTransactionsProof {
    let blockchain = blockchain.read();

    let mut verifier_state = None;
    let election_head = blockchain.election_head().block_number();
    let macro_head = blockchain.macro_head().block_number();

    // Get the historic transaction from the history store
    let mut historic_transactions = blockchain
        .history_store
        .get_hist_tx_by_hash(transaction, None);

    // Due to the history store implementation, potentially, we could have multiple historic transactions at this hash
    // So we just pick any transaction
    if let Some(hist_txn) = historic_transactions.pop() {
        let block_number = hist_txn.block_number;

        let proving_block_number = if block_number <= election_head {
            // If the txn is in a finalized epoch, we use the last election block
            election_head
        } else if block_number <= macro_head {
            // If the txn is in a finalized batch in the current epoch, we use the last checkpoint block
            macro_head
        } else {
            // If the txn is in the current batch, we use the transaction's block
            hist_txn.block_number
        };

        let block = blockchain
            .chain_store
            .get_block_at(proving_block_number, false, None)
            .ok();

        if let Some(ref block) = block {
            // If it is a checkpoint block, we have some extra work to do
            if Policy::is_macro_block_at(proving_block_number)
                && !Policy::is_election_block_at(proving_block_number)
            {
                let chain_info = blockchain.get_chain_info(&block.hash(), false, None);
                let history_tree_len = chain_info.unwrap().history_tree_len;
                verifier_state = Some(history_tree_len as usize);
            }
        } else {
            // If we couldn't find the block, then we cannot prove the transaction
            return ResponseTransactionsProof {
                proof: None,
                block: None,
            };
        }

        // Prove the transaction
        let proof = blockchain.history_store.prove(
            Policy::epoch_at(proving_block_number),
            vec![transaction],
            verifier_state,
            None,
        );

        // If we couldn't obtain a proof we return None
        if proof.is_none() {
            log::info!("Could not generate the txn inclusion proof");
            return ResponseTransactionsProof {
                proof: None,
                block: None,
            };
        }

        ResponseTransactionsProof { proof, block }
    } else {
        // If we couldn't find the transaction in our history store, then we cannot prove it.
        ResponseTransactionsProof {
            proof: None,
            block: None,
        }
    }
}

#[cfg(feature = "full")]
impl<N: Network> Handle<N, Arc<RwLock<Blockchain>>> for RequestTransactionsProof {
    fn handle(
        &self,
        _peer_id: N::PeerId,
        blockchain: &Arc<RwLock<Blockchain>>,
    ) -> ResponseTransactionsProof {
        if self.hashes.is_empty() {
            // If we are not given a list of transactions then there is nothing to do
            return ResponseTransactionsProof {
                proof: None,
                block: None,
            };
        }

        // Handle the different cases: if we are provided a block number (to generate the proof) or not
        if let Some(block_number) = self.block_number {
            prove_txns_with_block_number(blockchain, &self.hashes, block_number)
        } else {
            prove_transaction(blockchain, &self.hashes[0])
        }
    }
}

#[cfg(feature = "full")]
impl<N: Network> Handle<N, Arc<RwLock<Blockchain>>> for RequestTransactionReceiptsByAddress {
    fn handle(
        &self,
        _peer_id: N::PeerId,
        blockchain: &Arc<RwLock<Blockchain>>,
    ) -> ResponseTransactionReceiptsByAddress {
        let blockchain = blockchain.read();

        // Get the transaction hashes for this address.
        let tx_hashes = blockchain.history_store.get_tx_hashes_by_address(
            &self.address,
            self.max.unwrap_or(500).min(500),
            None,
        );

        let mut receipts = vec![];

        for hash in tx_hashes {
            // Get all the historic transactions that correspond to this hash.
            receipts.extend(
                blockchain
                    .history_store
                    .get_hist_tx_by_hash(&hash, None)
                    .iter()
                    .map(|hist_tx| (hist_tx.tx_hash(), hist_tx.block_number)),
            );
        }

        ResponseTransactionReceiptsByAddress { receipts }
    }
}

#[cfg(feature = "full")]
impl<N: Network> Handle<N, Arc<RwLock<Blockchain>>> for RequestTrieProof {
    fn handle(
        &self,
        _peer_id: N::PeerId,
        blockchain: &Arc<RwLock<Blockchain>>,
    ) -> ResponseTrieProof {
        let blockchain = blockchain.read();

        // We only prove accounts that exist in our current state
        let proof = blockchain.get_accounts_proof(self.keys.iter().collect());

        if proof.is_none() {
            // If we could not generate a proof we respond with an empty result
            return ResponseTrieProof {
                proof: None,
                block_hash: None,
            };
        }

        let block_hash = blockchain.head_hash();

        ResponseTrieProof {
            proof,
            block_hash: Some(block_hash),
        }
    }
}

#[cfg(feature = "full")]
impl<N: Network> Handle<N, Arc<RwLock<Blockchain>>> for RequestBlocksProof {
    fn handle(
        &self,
        _peer_id: N::PeerId,
        blockchain: &Arc<RwLock<Blockchain>>,
    ) -> ResponseBlocksProof {
        let blockchain = blockchain.read();

        // Check if the request is sane and we can answer it
        for &block_number in &self.blocks {
            if !Policy::is_election_block_at(block_number)
                || block_number > self.election_head
                || self.election_head > blockchain.election_head().block_number()
            {
                return ResponseBlocksProof { proof: None };
            }
        }

        // Collect all election blocks needed for the proof
        let mut election_numbers = Vec::new();
        let mut block_proof = Vec::new();
        for block_number in &self.blocks {
            let hops = BlockInclusionProof::get_interlink_hops(*block_number, self.election_head);
            let mut hop_blocks = Vec::new();
            for &hop in &hops {
                if !election_numbers.contains(&hop) {
                    if let Ok(Block::Macro(hop_block)) = blockchain.get_block_at(hop, false, None) {
                        hop_blocks.push(hop_block);
                    } else {
                        continue;
                    }
                }
            }
            election_numbers.extend_from_slice(&hops);
            block_proof.append(&mut hop_blocks);
        }

        ResponseBlocksProof {
            proof: Some(BlockInclusionProof { proof: block_proof }),
        }
    }
}
