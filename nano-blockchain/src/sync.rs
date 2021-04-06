use nimiq_block_albatross::{Block, BlockError};
use nimiq_blockchain_albatross::{AbstractBlockchain, ChainInfo, PushError, PushResult};
use nimiq_nano_sync::{NanoProof, NanoZKP};
use nimiq_primitives::policy;

use crate::blockchain::NanoBlockchain;

/// Implements methods to sync a nano node.
impl NanoBlockchain {
    /// Syncs using a zero-knowledge proof. It receives an election block and a proof that there is
    /// a valid chain between the genesis block and that block.
    /// This brings the node from the genesis block all the way to the most recent election block.
    /// It is the default way to sync for a nano node.
    pub fn push_zkp(&mut self, block: Block, proof: NanoProof) -> Result<PushResult, PushError> {
        // Must be an election block.
        assert!(block.is_election());

        // Check the version
        if block.header().version() != policy::VERSION {
            return Err(PushError::InvalidBlock(BlockError::UnsupportedVersion));
        }

        // Checks if the body exists.
        let body = block
            .body()
            .ok_or(PushError::InvalidBlock(BlockError::MissingBody))?;

        // Check the body root.
        if &body.hash() != block.header().body_root() {
            return Err(PushError::InvalidBlock(BlockError::BodyHashMismatch));
        }

        // Prepare the inputs to verify the proof.
        let initial_block_number = self.genesis_block.block_number();

        let initial_header_hash = <[u8; 32]>::from(self.genesis_block.hash());

        let initial_public_keys = self
            .genesis_block
            .validators()
            .unwrap()
            .to_pks()
            .iter()
            .map(|pk| pk.public_key)
            .collect();

        let final_block_number = block.block_number();

        let final_header_hash = <[u8; 32]>::from(block.hash());

        let final_public_keys = block
            .validators()
            .unwrap()
            .to_pks()
            .iter()
            .map(|pk| pk.public_key)
            .collect();

        // Verify the zk proof.
        let verify_result = NanoZKP::verify(
            initial_block_number,
            initial_header_hash,
            initial_public_keys,
            final_block_number,
            final_header_hash,
            final_public_keys,
            proof,
        );

        if verify_result.is_err() || !verify_result.unwrap() {
            return Err(PushError::InvalidZKP);
        }

        // At this point we know that the block is correct. We just have to push it.

        // Get write transaction for ChainStore.
        let mut chain_store_w = self
            .chain_store
            .write()
            .expect("Couldn't acquire write lock to ChainStore!");

        // Create the chain info for the new block.
        let chain_info = ChainInfo::new(block.clone(), true);

        // Since it's a macro block, we have to clear the ChainStore. If we are syncing for the first
        // time, this should be empty. But we clear it just in case it's not our first time.
        chain_store_w.clear();

        // Store the block chain info.
        chain_store_w.put_chain_info(chain_info);

        // Store the election block header.
        chain_store_w.put_election(block.unwrap_macro_ref().header.clone());

        // Update the blockchain.
        self.head = block.clone();

        self.macro_head = block.clone().unwrap_macro();

        self.election_head = block.clone().unwrap_macro();

        self.current_validators = block.validators();

        Ok(PushResult::Extended)
    }

    /// Pushes a macro block into the blockchain. This is used when we have already synced to the
    /// most recent election block and now need to push a checkpoint block.
    /// But this function is general enough to allow pushing any macro block (checkpoint or election)
    /// at any state of the node (synced, partially synced, not synced).
    pub fn push_macro(&mut self, block: Block) -> Result<PushResult, PushError> {
        // Must be a macro block.
        assert!(block.is_macro());

        // Check the version
        if block.header().version() != policy::VERSION {
            return Err(PushError::InvalidBlock(BlockError::UnsupportedVersion));
        }

        // If this is an election block, check the body.
        if block.is_election() {
            // Checks if the body exists.
            let body = block
                .body()
                .ok_or(PushError::InvalidBlock(BlockError::MissingBody))?;

            // Check the body root.
            if &body.hash() != block.header().body_root() {
                return Err(PushError::InvalidBlock(BlockError::BodyHashMismatch));
            }
        }

        // Check if we have this block's parent. The checks change depending if the last macro block
        // that we pushed was an election block or not.
        if policy::is_election_block_at(self.block_number()) {
            // We only need to check that the parent election block of this block is the same as our
            // head block.
            if block.header().parent_election_hash().unwrap() != &self.head_hash() {
                return Err(PushError::Orphan);
            }
        } else {
            // We need to check that this block and our head block have the same parent election
            // block and are in the correct order.
            if block.header().parent_election_hash().unwrap()
                != self.head().parent_election_hash().unwrap()
                || block.block_number() <= self.head.block_number()
            {
                return Err(PushError::Orphan);
            }
        }

        // Checks if the justification exists.
        let justification = block
            .unwrap_macro_ref()
            .justification
            .as_ref()
            .ok_or(PushError::InvalidBlock(BlockError::NoJustification))?;

        // Verify the justification.
        if !justification.verify(
            block.hash(),
            block.block_number(),
            &self.current_validators().unwrap(),
        ) {
            return Err(PushError::InvalidBlock(BlockError::InvalidJustification));
        }

        // At this point we know that the block is correct. We just have to push it.

        // Get write transaction for ChainStore.
        let mut chain_store_w = self
            .chain_store
            .write()
            .expect("Couldn't acquire write lock to ChainStore!");

        // Create the chain info for the new block.
        let chain_info = ChainInfo::new(block.clone(), true);

        // Since it's a macro block, we have to clear the ChainStore.
        chain_store_w.clear();

        // Store the block chain info.
        chain_store_w.put_chain_info(chain_info);

        // Update the blockchain.
        self.head = block.clone();

        self.macro_head = block.clone().unwrap_macro();

        // If it's an election block, you have more steps.
        if block.is_election() {
            self.election_head = block.unwrap_macro_ref().clone();

            self.current_validators = block.validators();

            // Store the election block header.
            chain_store_w.put_election(block.unwrap_macro().header);
        }

        Ok(PushResult::Extended)
    }
}
