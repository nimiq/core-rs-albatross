use std::time::Instant;

use database::{Transaction, ReadTransaction};
use hash::Blake2bHash;
use network_primitives::networks::get_network_info;
use block::{Block, BlockHeader, Target};
use block::proof::ChainProof;
use utils::iterators::Merge;

use crate::{Blockchain, chain_info::ChainInfo};

impl<'env> Blockchain<'env> {
    const NIPOPOW_M: u32 = 240;
    const NIPOPOW_K: u32 = 120;
    const NIPOPOW_DELTA: f64 = 0.15;

    pub fn get_chain_proof(&self) -> ChainProof {
        let mut state = self.state.write();
        if state.chain_proof.is_none() {
            let start = Instant::now();
            let chain_proof = self.prove(&state.main_chain.head, Self::NIPOPOW_M, Self::NIPOPOW_K, Self::NIPOPOW_DELTA);
            trace!("Chain proof took {}ms to compute (prefix={}, suffix={})", utils::time::duration_as_millis(&(Instant::now() - start)), chain_proof.prefix.len(), chain_proof.suffix.len());
            state.chain_proof = Some(chain_proof);
        }
        // XXX Get rid of the clone here? ChainProof is typically >1mb.
        state.chain_proof.as_ref().unwrap().clone()
    }

    fn prove(&self, head: &Block, m: u32, k: u32, delta: f64) -> ChainProof {
        let mut prefix = vec![];
        let mut start_height = 1u32;

        let txn = ReadTransaction::new(self.env);
        let head_info = self.chain_store
            .get_chain_info_at(u32::max(head.header.height.saturating_sub(k), 1), false, Some(&txn))
            .expect("Failed to compute chain proof - prefix head block not found");
        let max_depth = head_info.super_block_counts.get_candidate_depth(m);

        for depth in (0..=max_depth).rev() {
            let super_chain = self.get_super_chain(depth, &head_info, start_height, Some(&txn));
            if super_chain.is_good(depth, m, delta) {
                assert!(super_chain.0.len() >= m as usize, "Good superchain too short");
                trace!("Found good superchain at depth {} with length {} (#{} - #{})", depth, super_chain.0.len(), start_height, head_info.head.header.height);
                start_height = super_chain.0[super_chain.0.len() - m as usize].head.header.height;
            }

            let merged = Merge::new(
                prefix.into_iter(),
                super_chain.0.into_iter().map(|chain_info| chain_info.head),
                |l, r| u32::cmp(&l.header.height, &r.header.height));
            prefix = merged.collect();
        }

        let suffix = self.get_header_chain(head.header.height - head_info.head.header.height, &head, Some(&txn));

        ChainProof { prefix, suffix }
    }

    fn get_super_chain(&self, depth: u8, head_info: &ChainInfo, tail_height: u32, txn_option: Option<&Transaction>) -> SuperChain {
        assert!(tail_height >= 1, "Tail height must be >= 1");
        let mut chain = vec![];

        // Include head if it is at the requested depth or below.
        let head_depth = Target::from(&head_info.head.header.pow()).get_depth();
        if head_depth >= depth {
            chain.push(head_info.clone());
        }

        let mut block;
        let mut head = &head_info.head;
        let mut j = i16::max(i16::from(depth) - i16::from(Target::from(head.header.n_bits).get_depth()), -1);
        while j < head.interlink.hashes.len() as i16 && head.header.height > tail_height {
            let reference = if j < 0 {
                &head.header.prev_hash
            } else {
                &head.interlink.hashes[j as usize]
            };

            let chain_info = self.chain_store
                .get_chain_info(reference, false, txn_option)
                .expect("Failed to construct superchain - missing block");
            block = chain_info.head.clone();
            chain.push(chain_info);

            head = &block;
            j = i16::max(i16::from(depth) - i16::from(Target::from(head.header.n_bits).get_depth()), -1);
        }

        if (chain.is_empty() || chain[chain.len() - 1].head.header.height > 1) && tail_height == 1 {
            let genesis_block = get_network_info(self.network_id).unwrap().genesis_block.clone();
            chain.push(ChainInfo::initial(genesis_block.into_light()));
        }

        chain.reverse();
        SuperChain(chain)
    }

    fn get_header_chain(&self, length: u32, head: &Block, txn_option: Option<&Transaction>) -> Vec<BlockHeader> {
        let mut headers = vec![];

        if length > 0 {
            headers.push(head.header.clone());
        }

        let mut prev_hash = head.header.prev_hash.clone();
        let mut height = head.header.height;
        while headers.len() < length as usize && height > 1 {
            let block = self.chain_store
                .get_block(&prev_hash, false, txn_option)
                .expect("Failed to construct header chain - missing block");

            prev_hash = block.header.prev_hash.clone();
            height = block.header.height;

            headers.push(block.header);
        }

        headers.reverse();
        headers
    }

    pub fn get_block_proof(&self, hash_to_prove: &Blake2bHash, known_hash: &Blake2bHash) -> Option<Vec<Block>> {
        let txn = ReadTransaction::new(self.env);

        let block_to_prove = self.chain_store
            .get_block(hash_to_prove, false, Some(&txn))?;

        if hash_to_prove == known_hash || block_to_prove.header.height == 1 {
            return Some(vec![]);
        }

        let mut block = self.chain_store
            .get_block(known_hash, false, Some(&txn))?;

        let mut blocks = vec![];
        let mut depth = i16::from(Target::from(block.header.n_bits).get_depth()) + block.interlink.len() as i16 - 1;
        let prove_depth = i16::from(Target::from(&block_to_prove.header.pow()).get_depth());

        let index = i16::min(depth - i16::from(Target::from(block.header.n_bits).get_depth()), block.interlink.len() as i16 - 1);
        let mut reference = if index < 0 {
            &block.header.prev_hash
        } else {
            &block.interlink.hashes[index as usize]
        };

        while hash_to_prove != reference && block.header.height > 1 {
            let next_block = self.chain_store
                .get_block(reference, false, Some(&txn))
                .expect("Failed to construct block proof - missing block");

            if next_block.header.height < block_to_prove.header.height {
                // We have gone past the blockToProve, but are already at proveDepth, fail.
                if depth <= prove_depth {
                    return None;
                }

                // Decrease depth and thereby step size.
                depth -= 1;
            } else if next_block.header.height > block_to_prove.header.height {
                // We are still in front of block_to_prove, add block to result and advance.
                blocks.push(next_block.clone());
                block = next_block;
            } else {
                // We found a reference to a different block than blockToProve at its height.
                warn!("Failed to prove block {} - different block {} at its height {}", hash_to_prove, reference, block_to_prove.header.height);
                return None;
            }

            let index = i16::min(depth - i16::from(Target::from(block.header.n_bits).get_depth()), block.interlink.len() as i16 - 1);
            reference = if index < 0 {
                &block.header.prev_hash
            } else {
                &block.interlink.hashes[index as usize]
            };
        }

        // Include the block_to_prove in the result.
        blocks.push(block_to_prove);
        blocks.reverse();
        Some(blocks)
    }
}

struct SuperChain(Vec<ChainInfo>);
impl SuperChain {
    pub fn is_good(&self, depth: u8, m: u32, delta: f64) -> bool {
        self.has_super_quality(depth, m, delta) && self.has_multi_level_quality(depth, m, delta)
    }

    fn has_super_quality(&self, depth: u8, m: u32, delta: f64) -> bool {
        let length = self.0.len();
        if length < m as usize {
            return false;
        }

        for i in m as usize..=length {
            let underlying_length = self.0[length - 1].head.header.height - self.0[length - i].head.header.height + 1;
            if !SuperChain::is_locally_good(i as u32, underlying_length, depth, delta) {
                return false;
            }
        }

        true
    }

    fn has_multi_level_quality(&self, depth: u8, k1: u32, delta: f64) -> bool {
        if depth == 0 {
            return true;
        }

        for i in 0..(self.0.len() - k1 as usize) {
            let tail_info = &self.0[i];
            let head_info = &self.0[i + k1 as usize];

            for mu in (1..=depth).rev() {
                let upper_chain_length = head_info.super_block_counts.get(mu) - tail_info.super_block_counts.get(mu);

                // Moderate badness check:
                for j in (0..mu).rev() {
                    let lower_chain_length = head_info.super_block_counts.get(j) - tail_info.super_block_counts.get(j);
                    if !SuperChain::is_locally_good(upper_chain_length, lower_chain_length, mu - j, delta) {
                        trace!("Chain badness detected at depth {}[{}:{}], failing at {}/{}", depth, i, i + k1 as usize, mu, j);
                        return false;
                    }
                }
            }
        }

        true
    }

    fn is_locally_good(super_length: u32, underlying_length: u32, depth: u8, delta: f64) -> bool {
        f64::from(super_length) > (1f64 - delta) * 2f64.powi(-i32::from(depth)) * f64::from(underlying_length)
    }
}
