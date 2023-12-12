use itertools::Itertools;
use nimiq_hash::Blake2bHasher;
use nimiq_utils::merkle::compute_root_from_content;

use super::public_key::DelinearizedPublicKey;
use crate::{Address, PublicKey};

pub fn combine_public_keys(public_keys: Vec<PublicKey>, num_signers: usize) -> Vec<PublicKey> {
    // Calculate combinations.
    let combinations = public_keys.into_iter().combinations(num_signers);
    let mut multisig_keys: Vec<PublicKey> = combinations
        .map(|combination| DelinearizedPublicKey::sum_delinearized(&combination))
        .collect();
    multisig_keys.sort();
    multisig_keys
}

pub fn compute_address(combined_public_keys: &[PublicKey]) -> Address {
    // Calculate address.
    let merkle_root = compute_root_from_content::<Blake2bHasher, _>(combined_public_keys);
    Address::from(merkle_root)
}
