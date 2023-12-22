use std::{borrow::Borrow, iter::Sum};

use curve25519_dalek::{edwards::EdwardsPoint, traits::Identity};

use crate::{multisig::hash_public_keys, PublicKey};

/// This structure holds a delinearized public key (which prevents rogue key attacks in multisigs).
#[derive(Copy, Clone)]
pub struct DelinearizedPublicKey(EdwardsPoint);

impl DelinearizedPublicKey {
    /// Delinearizes a public key by multiplying it with a scalar derived from the hash and the public key itself.
    /// Effective delinearization for multisigs should use the hash over all public keys as an input.
    fn new(public_key: PublicKey, hash: &[u8; 64]) -> Self {
        DelinearizedPublicKey(public_key.delinearize(hash))
    }

    /// Delinearizes a list of public keys and returns the list of delinearized public keys.
    /// Delinearizaion prevents rogue key attacks.
    /// Each public key is multiplied with a scalar derived from the hash over all public keys and the public key itself.
    pub fn delinearize(public_keys: &[PublicKey]) -> Vec<Self> {
        let mut public_keys = public_keys.to_vec();
        public_keys.sort();
        let h = hash_public_keys(&public_keys);
        public_keys
            .into_iter()
            .map(|pk| DelinearizedPublicKey::new(pk, &h))
            .collect()
    }

    /// Delinearizes and aggregates a list of public keys.
    /// Delinearizaion prevents rogue key attacks.
    pub fn sum_delinearized(public_keys: &[PublicKey]) -> PublicKey {
        let d: DelinearizedPublicKey = DelinearizedPublicKey::delinearize(public_keys)
            .into_iter()
            .sum();
        d.into()
    }
}

impl From<DelinearizedPublicKey> for PublicKey {
    fn from(dpk: DelinearizedPublicKey) -> Self {
        PublicKey::from(dpk.0.compress().to_bytes())
    }
}

impl<T> Sum<T> for DelinearizedPublicKey
where
    T: Borrow<DelinearizedPublicKey>,
{
    fn sum<I>(iter: I) -> Self
    where
        I: Iterator<Item = T>,
    {
        DelinearizedPublicKey(
            iter.fold(EdwardsPoint::identity(), |acc, item| acc + item.borrow().0),
        )
    }
}
