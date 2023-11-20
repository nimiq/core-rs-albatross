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
//! 624300 -> 00000000000000000011bcc88fff08e31cda3af4b1f2cdf7e8ae119e2940c105
//!
//! 624301 -> 00000000000000000002173661019adb14422b4c32d84f9edf34e4e0479cc610
//!
//! 624302 -> 0000000000000000000fa6875de9b6ea8fa359f0c8aa880f3164538d60b8a74d
//!
//! 624303 -> 0000000000000000001132c69b93e3f778c003ea3c1bbe3bcef16f2dc8257c18
//!
//! 624304 -> 000000000000000000115c1cf2bc6fb5efc1a5ce2354443151b37fa8fad81d5d
//!
//! 624305 -> 00000000000000000004bd6c527a85654b0447c0342c1e78b68c45ec03586974
//!
//! 624306 -> 00000000000000000012be20464779c18b52f5f06d0f88ccec02e2b43f523787
//!
//! 624307 -> 00000000000000000012df8d6b970f89a9c4c4c85bc1eb3e6043935dc14cb8fd
//!
//! 624308 -> 00000000000000000010d570fd6e234859fc0c85f1a33b60119fec9e0d91c5c4
//!
//! 624309 -> 0000000000000000000efcca9f959bc3d159a884ba0cc2c74dd7ee9168990c7d
//!
//! 624310 -> 00000000000000000011ed57d95754efcf8564d84ae9e141d95f57758b81634e
//!
//! 624311 -> 0000000000000000000574c978fd19aeb0a16ca0b78a9bdd71e6b8b640511d68
//!
//! 624312 -> 0000000000000000000eb22a79e5bdc397ba32365471c14a974296d160b387aa
//!
//! 624313 -> 00000000000000000013eae18508584cd7a71b216841ed759698a7d14072cdb6
//!
//! 624314 -> 000000000000000000132006558a816203cb442aeb9162ba1d8f6dac5f0a00ec

use nimiq_hash::{Blake2bHash, Hash};

/// This function will return 32 verifiably random bytes.
pub fn generate_random_seed(personalization: u64) -> [u8; 32] {
    // This will contain the initial random hashes and convert them into bytes.
    let block_00 = "00000000000000000011bcc88fff08e31cda3af4b1f2cdf7e8ae119e2940c105";
    let block_01 = "00000000000000000002173661019adb14422b4c32d84f9edf34e4e0479cc610";
    let block_02 = "0000000000000000000fa6875de9b6ea8fa359f0c8aa880f3164538d60b8a74d";
    let block_03 = "0000000000000000001132c69b93e3f778c003ea3c1bbe3bcef16f2dc8257c18";
    let block_04 = "000000000000000000115c1cf2bc6fb5efc1a5ce2354443151b37fa8fad81d5d";
    let block_05 = "00000000000000000004bd6c527a85654b0447c0342c1e78b68c45ec03586974";
    let block_06 = "00000000000000000012be20464779c18b52f5f06d0f88ccec02e2b43f523787";
    let block_07 = "00000000000000000012df8d6b970f89a9c4c4c85bc1eb3e6043935dc14cb8fd";
    let block_08 = "00000000000000000010d570fd6e234859fc0c85f1a33b60119fec9e0d91c5c4";
    let block_09 = "0000000000000000000efcca9f959bc3d159a884ba0cc2c74dd7ee9168990c7d";
    let block_10 = "00000000000000000011ed57d95754efcf8564d84ae9e141d95f57758b81634e";
    let block_11 = "0000000000000000000574c978fd19aeb0a16ca0b78a9bdd71e6b8b640511d68";
    let block_12 = "0000000000000000000eb22a79e5bdc397ba32365471c14a974296d160b387aa";
    let block_13 = "00000000000000000013eae18508584cd7a71b216841ed759698a7d14072cdb6";
    let block_14 = "000000000000000000132006558a816203cb442aeb9162ba1d8f6dac5f0a00ec";

    let concatenated = format!(
        "{block_00}{block_01}{block_02}{block_03}{block_04}{block_05}{block_06}{block_07}{block_08}{block_09}{block_10}{block_11}{block_12}{block_13}{block_14}"
    );

    let mut random_bytes = hex::decode(concatenated).unwrap();
    random_bytes.extend_from_slice(&personalization.to_be_bytes());

    random_bytes.hash::<Blake2bHash>().0
}
