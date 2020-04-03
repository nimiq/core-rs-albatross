//! # Randomness Generation
//! ## Purpose
//! This file serves two functions:
//! 1) Hosting a tool that can be used to create a 256 bit random seed.
//! 2) Documenting in a public way the process so that it can be verified that, in fact,
//! the seed is random
//!
//! ## Method
//! The initial random bytes are going to be the concatenated hashes of 15 Bitcoin block headers
//! with the following numbers:
//!
//! 624300  624301  624302  624303  624304
//! 624305  624306  624307  624308  624309
//! 624310  624311  624312  624313  624314
//!
//! These blocks are going to be mined about a day from now. This should be enough to convince anyone
//! that: a) the hashes of those blocks are unknown to me at the time that I am writing this and b)
//! that I would need more than 51% of the Bitcoin network hashpower in order to control the hashes.
//!
//! ## Bitcoin block header hashes
//! We got the hashes for the following Bitcoin block headers:
//!
//! 624300 ->
//!
//! 624301 ->
//!
//! 624302 ->
//!
//! 624303 ->
//!
//! 624304 ->
//!
//! 624305 ->
//!
//! 624306 ->
//!
//! 624307 ->
//!
//! 624308 ->
//!
//! 624309 ->
//!
//! 624310 ->
//!
//! 624311 ->
//!
//! 624312 ->
//!
//! 624313 ->
//!
//! 624314 ->

use crypto_primitives::prf::Blake2sWithParameterBlock;

/// This function will return 32 verifiably random bytes.
pub fn generate_random_seed() -> Vec<u8> {
    // This will contain the initial random hashes and convert them into bytes.
    let block_01 = "";
    let block_02 = "";
    let block_03 = "";
    let block_04 = "";
    let block_05 = "";
    let block_06 = "";
    let block_07 = "";
    let block_08 = "";
    let block_09 = "";
    let block_10 = "";
    let block_11 = "";
    let block_12 = "";
    let block_13 = "";
    let block_14 = "";
    let block_15 = "";
    let concatenated = format!(
        "{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}",
        block_01,
        block_02,
        block_03,
        block_04,
        block_05,
        block_06,
        block_07,
        block_08,
        block_09,
        block_10,
        block_11,
        block_12,
        block_13,
        block_14,
        block_15
    );
    let random_bytes = hex::decode(concatenated).unwrap();

    // Initialize Blake2s parameters.
    let blake2s = Blake2sWithParameterBlock {
        digest_length: 32,
        key_length: 0,
        fan_out: 1,
        depth: 1,
        leaf_length: 0,
        node_offset: 0,
        xof_digest_length: 0,
        node_depth: 0,
        inner_length: 0,
        salt: [0; 8],
        personalization: [0; 8],
    };

    // Calculate the Blake2s hash.
    blake2s.evaluate(random_bytes.as_ref())
}
