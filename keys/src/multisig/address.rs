use itertools::Itertools;
#[cfg(feature = "serde-derive")]
use nimiq_hash::Blake2bHasher;
#[cfg(feature = "serde-derive")]
use nimiq_utils::merkle::compute_root_from_content;

use super::public_key::DelinearizedPublicKey;
#[cfg(feature = "serde-derive")]
use crate::Address;
use crate::PublicKey;

/// Generates all possible k-of-k multisig addresses (k = `num_signers`) from the list of `public_keys`.
pub fn combine_public_keys(public_keys: Vec<PublicKey>, num_signers: usize) -> Vec<PublicKey> {
    // Calculate combinations.
    let combinations = public_keys.into_iter().combinations(num_signers);
    let mut multisig_keys: Vec<PublicKey> = combinations
        .map(|combination| DelinearizedPublicKey::sum_delinearized(&combination))
        .collect();
    multisig_keys.sort();
    multisig_keys
}

/// Given a list of possible public keys, generates an address for which each of the public keys is a possible signer.
/// Our multisig scheme only allows n-of-n signatures. To achieve a k-of-n signature, we generate all possible combinations
/// for k-of-k signatures and compute the joint address using this method (see `combine_public_keys`).
#[cfg(feature = "serde-derive")]
pub fn compute_address(combined_public_keys: &[PublicKey]) -> Address {
    // Calculate address.
    let merkle_root = compute_root_from_content::<Blake2bHasher, _>(combined_public_keys);
    Address::from(merkle_root)
}
