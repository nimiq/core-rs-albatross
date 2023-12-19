use std::collections::HashMap;

use nimiq_primitives::policy::Policy;
use nimiq_utils::math::log2;
use serde::{Deserialize, Serialize};

use crate::MacroBlock;

// Block inclusion proofs proof that a block is part of the blockchain.
// The proof consists of an interlink chain from the current election head down to the target block.
#[derive(Clone, Serialize, Deserialize)]
pub struct BlockInclusionProof {
    pub proof: Vec<MacroBlock>,
}

impl BlockInclusionProof {
    // Computes the interlink hops needed to proof `wanted_election_number` when starting from `latest_election_number`
    pub fn get_interlink_hops(target: u32, latest_election_number: u32) -> Vec<u32> {
        // For simplicity, refer to election blocks by the number of their epoch
        let target_number = Policy::epoch_at(target);
        let latest_number = Policy::epoch_at(latest_election_number);

        // Compute the hops
        let mut hops = vec![];

        let mut current_hop = latest_number;
        let mut hop_contains_target_interlink = false;

        // We're done if the election parent is the target or if the block contains an interlink to the target
        while current_hop - 1 > target_number && !hop_contains_target_interlink {
            let previous_block_no = current_hop - 1;
            let interlink_count = (log2(previous_block_no as usize) as f32).floor() as u32;
            for i in (1..interlink_count + 1).rev() {
                let interlink_divider = 2_u32.pow(i);
                let ith_interlink = ((previous_block_no / interlink_divider) as f32).floor()
                    * interlink_divider as f32;
                if ith_interlink == target_number as f32 {
                    hop_contains_target_interlink = true;
                    break;
                }
                if ith_interlink > target_number as f32 {
                    current_hop = ith_interlink as u32;
                    hops.push(current_hop);
                    break;
                }
            }
        }

        // Convert hops back from epoch to block numbers
        hops.into_iter()
            .map(|i| Policy::election_block_of(i).expect("Invalid epoch number"))
            .collect()
    }

    // Checks whether the BlockInclusionProof proofs `target` when starting from `election_head`
    pub fn is_block_proven(&self, election_head: &MacroBlock, target: &MacroBlock) -> bool {
        if !target.is_election_block() {
            return false;
        }

        // Check that the block proof is sane
        for block in &self.proof {
            if block.header.interlink.is_none() {
                return false;
            }
        }

        // Convert into a more suitable data structure TODO mabye just always use this and only convert to vec on wire
        let mut number_to_block = HashMap::<u32, MacroBlock>::new();
        for block in self.proof.clone() {
            number_to_block.insert(block.block_number(), block);
        }

        let hops = BlockInclusionProof::get_interlink_hops(
            target.block_number(),
            election_head.block_number(),
        );

        // The election head might already have an interlink or prev_election_head link to the target
        if hops.is_empty() {
            return true;
        }

        // Check that the proof contains all needed hops
        for hop_number in &hops {
            if !number_to_block.contains_key(hop_number) {
                return false;
            }
        }

        // Verify that our election head contains an interlink to the first hop
        let first_hop = &number_to_block[&hops[0]];
        if !election_head
            .header
            .interlink
            .as_ref()
            .unwrap()
            .contains(&first_hop.hash())
        {
            return false;
        }

        // Verify that all hops are present and reference the next one
        for i in 0..hops.len() - 1 {
            let current_block = &number_to_block[&hops[i]];
            let next_hop = &number_to_block[&hops[i + 1]];

            if let Some(interlink) = &current_block.header.interlink {
                if !interlink.contains(&next_hop.hash()) {
                    return false;
                }
            }
        }

        // Verify that the last hop references the target block (as interlink or parent_hash)
        let last_hop_number = &hops[hops.len() - 1];
        let last_hop = &number_to_block[last_hop_number];
        let interlink_contains_target = &last_hop
            .header
            .interlink
            .as_ref()
            .unwrap()
            .contains(&target.hash());
        if !interlink_contains_target && last_hop.header.parent_election_hash != target.hash() {
            return false;
        }
        true
    }
}
