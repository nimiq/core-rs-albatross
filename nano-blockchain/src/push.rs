use nimiq_block_albatross::{Block, BlockType};
use nimiq_blockchain_albatross::{
    AbstractBlockchain, Blockchain, ChainInfo, ChainOrdering, PushError, PushResult,
};

use crate::blockchain::NanoBlockchain;

/// Implements methods to push blocks into the chain. This is used when the node has already synced
/// and is just receiving newly produced blocks. It is also used for the final phase of syncing,
/// when the node is just receiving micro blocks.
impl NanoBlockchain {
    /// Pushes a block into the chain.
    pub fn push(&mut self, block: Block) -> Result<PushResult, PushError> {
        // Check if we already know this block.
        if self.get_chain_info(&block.hash(), false, None).is_some() {
            return Ok(PushResult::Known);
        }

        // Check if we have this block's parent.
        // If it's a macro block, we don't care about this since we will clear out the chain store anyway.
        let prev_info = self
            .get_chain_info(&block.parent_hash(), false, None)
            .ok_or_else(|| PushError::Orphan)?;

        // Calculate chain ordering.
        let chain_order = ChainOrdering::order_chains(self, &block, &prev_info, None);

        // If it is an inferior chain, we ignore it as it cannot become better at any point in time.
        if chain_order == ChainOrdering::Inferior {
            return Ok(PushResult::Ignored);
        }

        // Get the intended slot owner.
        let (validator, _) = self
            .get_slot_owner_at(block.block_number(), block.view_number(), None)
            .expect("Failed to find slot owner!");

        let intended_slot_owner = validator.public_key.uncompress_unchecked();

        // Check the header.
        Blockchain::verify_block_header(self, &block.header(), &intended_slot_owner, None)?;

        // Check the justification.
        Blockchain::verify_block_justification(
            self,
            &block.header(),
            &block.justification(),
            &intended_slot_owner,
            None,
        )?;

        // Create the chaininfo for the new block.
        let chain_info = match ChainInfo::from_block(block, &prev_info) {
            Ok(v) => v,
            Err(_) => {
                return Err(PushError::InvalidSuccessor);
            }
        };

        // More chain ordering.
        match chain_order {
            ChainOrdering::Extend => {
                return self.extend(chain_info, prev_info);
            }
            ChainOrdering::Better => {
                return self.rebranch(chain_info, prev_info);
            }
            ChainOrdering::Inferior => unreachable!(),
            ChainOrdering::Unknown => {}
        }

        // Otherwise, we are creating/extending a fork. Store ChainInfo.
        self.chain_store.write().unwrap().put_chain_info(chain_info);

        Ok(PushResult::Forked)
    }

    /// Extends the current main chain.
    fn extend(
        &mut self,
        mut chain_info: ChainInfo,
        mut prev_info: ChainInfo,
    ) -> Result<PushResult, PushError> {
        // Update chain infos.
        chain_info.on_main_chain = true;
        prev_info.main_chain_successor = Some(chain_info.head.hash());

        // If it's a macro block then we need to clear the ChainStore (since we only want to keep
        // the current batch in memory). Otherwise, we need to update the previous ChainInfo.
        if chain_info.head.is_macro() {
            self.chain_store
                .write()
                .expect("Couldn't acquire write lock for ChainStore!")
                .clear();
        } else {
            self.chain_store
                .write()
                .expect("Couldn't acquire write lock for ChainStore!")
                .put_chain_info(prev_info);
        }

        // Store the current chain info.
        self.chain_store
            .write()
            .expect("Couldn't acquire write lock for ChainStore!")
            .put_chain_info(chain_info.clone());

        // Update the head of the blockchain.
        self.head = chain_info.head.clone();

        // If the block is a macro block then we also need to update the macro head.
        if let Block::Macro(macro_block) = chain_info.head {
            self.macro_head = macro_block.clone();

            // If the block is also an election block, then we have more fields to update.
            if macro_block.is_election_block() {
                self.election_head = macro_block.clone();

                self.current_validators = macro_block.get_validators();
            }
        }

        Ok(PushResult::Extended)
    }

    /// Rebranches the current main chain.
    fn rebranch(
        &mut self,
        mut chain_info: ChainInfo,
        mut prev_info: ChainInfo,
    ) -> Result<PushResult, PushError> {
        // You can't rebranch a macro block.
        assert!(chain_info.head.is_micro());

        // Update chain infos.
        chain_info.on_main_chain = true;
        prev_info.main_chain_successor = Some(chain_info.head.hash());

        // Find the common ancestor between our current main chain and the fork chain.
        // Walk up the fork chain until we find a block that is part of the main chain.
        // Update the fork chain along the way.
        let mut current = prev_info;

        while !current.on_main_chain {
            // A fork can't contain a macro block. We already received that macro block, thus it must be on our
            // main chain.
            assert_eq!(
                current.head.ty(),
                BlockType::Micro,
                "Fork contains macro block"
            );

            // Get previous chain info.
            let prev_info = self
                .get_chain_info(current.head.parent_hash(), false, None)
                .expect("Corrupted store: Failed to find fork predecessor while rebranching");

            // Update the chain info.
            current.on_main_chain = true;

            // Store the chain info.
            self.chain_store
                .write()
                .expect("Couldn't acquire write lock to ChainStore!")
                .put_chain_info(current);

            current = prev_info;
        }

        // Remember the ancestor block.
        let ancestor = current;

        // Go back from the head of the forked chain until the ancestor, updating it along the way.
        current = self
            .get_chain_info(&self.head_hash(), false, None)
            .expect("Couldn't find the head chain info!");

        while current != ancestor {
            // Get previous chain info.
            let prev_info = self
                .get_chain_info(current.head.parent_hash(), false, None)
                .expect("Corrupted store: Failed to find fork predecessor while rebranching");

            // Update the chain info.
            current.on_main_chain = false;

            // Store the chain info.
            self.chain_store
                .write()
                .expect("Couldn't acquire write lock to ChainStore!")
                .put_chain_info(current);

            current = prev_info;
        }

        // Update the head of the blockchain.
        self.head = chain_info.head;

        Ok(PushResult::Rebranched)
    }
}
