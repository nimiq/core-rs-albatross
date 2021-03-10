use std::cmp;
use std::cmp::Ordering;

use nimiq_block_albatross::{Block, BlockType};
use nimiq_database::Transaction;

use crate::chain_info::ChainInfo;
use crate::AbstractBlockchain;

/// Enum describing all the possible ways of comparing one chain to the main chain.
#[derive(Debug, Eq, PartialEq)]
pub enum ChainOrdering {
    // This chain is an extension of the main chain.
    Extend,
    // This chain is better than the main chain.
    Better,
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
        } else {
            // To compare two blocks, we need to compare the view number at the intersection.
            //   [2] - [2] - [3] - [4]
            //      \- [3] - [3] - [3]
            // The example above illustrates that you actually want to choose the lower chain,
            // since its view change happened way earlier.
            // Let's thus find the first block on the branch (not on the main chain).
            // If there is a malicious fork, it might happen that the two view numbers before
            // the branch are the same. Then, we need to follow and compare.
            let mut view_numbers = vec![block.view_number()];

            let mut current = ChainInfo::new(block.clone(), false);

            let mut prev = prev_info.clone();

            while !prev.on_main_chain {
                // Macro blocks are final
                assert_eq!(
                    prev.head.ty(),
                    BlockType::Micro,
                    "Trying to rebranch across macro block"
                );

                view_numbers.push(prev.head.view_number());

                let prev_hash = prev.head.parent_hash();

                let prev_info = blockchain
                    .get_chain_info(prev_hash, false, txn_option)
                    .expect("Corrupted store: Failed to find fork predecessor while rebranching");

                current = prev;

                prev = prev_info;
            }

            // Now follow the view numbers back until you find one that differs.
            // Example:
            // [0] - [0] - [1]  *correct chain*
            //    \- [0] - [0]
            // Otherwise take the longest:
            // [0] - [0] - [1] - [0]  *correct chain*
            //    \- [0] - [1]
            let current_height = current.head.block_number();
            let min_height = cmp::min(blockchain.block_number(), block.block_number());

            // Iterate over common block heights starting from right after the intersection.
            for h in current_height..=min_height {
                // Take corresponding view number from branch.
                let branch_view_number = view_numbers.pop().unwrap();

                // And calculate equivalent on main chain.
                let current_on_main_chain = blockchain
                    .get_block_at(h, false, txn_option)
                    .expect("Corrupted store: Failed to find main chain equivalent of fork");

                // Choose better one as early as possible.
                match current_on_main_chain.view_number().cmp(&branch_view_number) {
                    Ordering::Less => {
                        chain_order = ChainOrdering::Better;
                        break;
                    }
                    Ordering::Greater => {
                        chain_order = ChainOrdering::Inferior;
                        break;
                    }
                    Ordering::Equal => {} // Continue...
                }
            }

            // If they were all equal, choose the longer one.
            if chain_order == ChainOrdering::Unknown
                && blockchain.block_number() < block.block_number()
            {
                chain_order = ChainOrdering::Better;
            }

            info!(
                "New block is on {:?} chain with fork at #{} (current #{}.{}, new block #{}.{})",
                chain_order,
                current_height - 1,
                blockchain.block_number(),
                blockchain.view_number(),
                block.block_number(),
                block.view_number()
            );
        }

        chain_order
    }
}
