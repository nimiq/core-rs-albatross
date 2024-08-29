use std::cmp;

use nimiq_block::Block;
use nimiq_hash::Blake2bHash;

use crate::{AbstractBlockchain, BlockchainError, ChainInfo};

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
    /// F and G functions are what the blockchain uses to obtain the chain_info and a block respectively
    /// They are abstracted in such a way, because the regular blockchain uses a DB transaction
    /// whereas the light blockchain does not, but they share all the same logic
    pub fn order_chains<B: AbstractBlockchain, F, G>(
        blockchain: &B,
        block: &Block,
        prev_info: &ChainInfo,
        get_chain_info: F,
        get_block_at: G,
    ) -> ChainOrdering
    where
        F: Fn(&Blake2bHash) -> Result<ChainInfo, BlockchainError>,
        G: Fn(u32) -> Result<Block, BlockchainError>,
    {
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
            let mut skip_block_flags = vec![block.is_skip()];
            let mut start_block_number = block.block_number();

            if !prev_info.on_main_chain {
                skip_block_flags.push(prev_info.head.is_skip());
                start_block_number = prev_info.head.block_number();
                let mut prev_chain_info = get_chain_info(prev_info.head.parent_hash())
                    .expect("Corrupted store: Failed to find fork predecessor");

                while !prev_chain_info.on_main_chain {
                    skip_block_flags.push(prev_chain_info.head.is_skip());
                    start_block_number = prev_chain_info.head.block_number();
                    prev_chain_info = get_chain_info(prev_chain_info.head.parent_hash())
                        .expect("Corrupted store: Failed to find fork predecessor");
                }
            }

            // Now go forward from the first block on the branch and detect which chain has an earlier
            // skip block.
            // Example (skip blocks noted as '1'):
            // [0] - [0] - [1]  *correct chain*
            //    \- [0] - [0]
            // Otherwise take the longest:
            // [0] - [0] - [1] - [0]  *correct chain*
            //    \- [0] - [1]
            let end_block_number = cmp::min(blockchain.block_number(), block.block_number());

            // Iterate over common block heights starting from right after the intersection.
            for h in start_block_number..=end_block_number {
                let block_is_skip = skip_block_flags.pop().unwrap();

                // Get equivalent block on main chain.
                let current_on_main_chain = get_block_at(h)
                    .expect("Corrupted store: Failed to find main chain equivalent of fork");

                if block_is_skip && !current_on_main_chain.is_skip() {
                    chain_order = ChainOrdering::Superior;
                    break;
                } else if !block_is_skip && current_on_main_chain.is_skip() {
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

            log::info!(
                fork_block_number = start_block_number - 1,
                current_block_number = blockchain.block_number(),
                new_block_number = block.block_number(),
                "New block in {:?} chain",
                chain_order
            );
        }

        chain_order
    }
}
