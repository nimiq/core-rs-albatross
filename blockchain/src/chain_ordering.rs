use std::cmp;

use nimiq_block::{Block, BlockType};
use nimiq_database::Transaction;

use crate::chain_info::ChainInfo;
use crate::AbstractBlockchain;

/// Enum describing all the possible ways of comparing one chain to the main chain.
#[derive(Debug, Eq, PartialEq)]
pub enum ChainOrdering {
    // This chain is an extension of the main chain.
    Extend,
    // This chain is better than the main chain.
    Superior,
    // This chain is worse than the main chain.
    Inferior,
    // The ordering of this chain is unknown.
    Unknown,
}

/// Implements method to calculate chain ordering.
impl ChainOrdering {
    /// Given a block and some chain, it returns the ordering of the new chain relative to the given
    /// chain.
    pub fn order_chains<B: AbstractBlockchain>(
        blockchain: &B,
        block: &Block,
        prev_info: &ChainInfo,
        txn_option: Option<&Transaction>,
    ) -> ChainOrdering {
        let mut chain_order = ChainOrdering::Unknown;

        if block.parent_hash() == &blockchain.head_hash() {
            chain_order = ChainOrdering::Extend;
        } else if block.is_macro() && block.block_number() > blockchain.block_number() {
            chain_order = ChainOrdering::Superior;
        } else {
            // To compare two chains, we need to compare and find which chain has an earlier
            // skip block. For example (numbers denotes accumulated skip block):
            //   [2] - [2] - [3] - [4]
            //      \- [3] - [3] - [3]
            // The example above illustrates that you actually want to choose the lower chain,
            // since its skip block happened way earlier.
            // Let's thus find the first block on the branch (not on the main chain).
            // If there is a malicious fork, it might happen that the two skip blocks before
            // the branch are the same. Then, we need to follow and compare.
            let mut blocks = vec![block.clone()];

            let mut current = ChainInfo::new(block.clone(), false);
            let mut prev = prev_info.clone();

            while !prev.on_main_chain {
                // Macro blocks are final
                assert!(
                    prev.head.ty() != BlockType::Macro,
                    "Trying to rebranch across macro block"
                );

                let prev_hash = prev.head.parent_hash();
                blocks.push(prev.head.clone());

                let prev_info = blockchain
                    .get_chain_info(prev_hash, false, txn_option)
                    .expect("Corrupted store: Failed to find fork predecessor while rebranching");

                current = prev;

                prev = prev_info;
            }

            // Now go forward from the first block on the branch and detect which chain has an earlier
            // skip block.
            // Example (skip blocks noted as '1'):
            // [0] - [0] - [1]  *correct chain*
            //    \- [0] - [0]
            // Otherwise take the longest:
            // [0] - [0] - [1] - [0]  *correct chain*
            //    \- [0] - [1]
            let current_height = current.head.block_number();
            let min_height = cmp::min(blockchain.block_number(), block.block_number());

            // Iterate over common block heights starting from right after the intersection.
            for h in current_height..=min_height {
                let current_block = blocks.pop().unwrap();

                // Get equivalent block on main chain.
                let current_on_main_chain = blockchain
                    .get_block_at(h, false, txn_option)
                    .expect("Corrupted store: Failed to find main chain equivalent of fork");

                if current_block.is_skip() && !current_on_main_chain.is_skip() {
                    chain_order = ChainOrdering::Superior;
                    break;
                } else if !current_block.is_skip() && current_on_main_chain.is_skip() {
                    chain_order = ChainOrdering::Inferior;
                    break;
                }
            }

            // If they were all equal, choose the longer one.
            if chain_order == ChainOrdering::Unknown
                && blockchain.block_number() < block.block_number()
            {
                chain_order = ChainOrdering::Superior;
            }

            info!(
                fork_block_number = current_height - 1,
                current_block_number = blockchain.block_number(),
                new_block_number = block.block_number(),
                "New block in {:?} chain",
                chain_order
            );
        }

        chain_order
    }
}
