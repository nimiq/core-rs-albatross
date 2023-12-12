use std::{borrow::Borrow, iter::Sum};

use curve25519_dalek::{
    edwards::{CompressedEdwardsY, EdwardsPoint},
    scalar::Scalar,
    traits::Identity,
};
use nimiq_hash::{sha512::Sha512Hasher, Hasher};

use crate::{multisig::hash_public_keys, PublicKey};

#[derive(Copy, Clone)]
pub struct DelinearizedPublicKey(EdwardsPoint);

impl DelinearizedPublicKey {
    fn new(public_key: PublicKey, hash: &[u8; 64]) -> Self {
        let pk_bytes = public_key.as_bytes();

        // Compute H(C||P).
        let mut hasher = Sha512Hasher::default();
        hasher.hash(hash);
        hasher.hash(pk_bytes);
        let hash = hasher.finish();
        let s = Scalar::from_bytes_mod_order_wide(&hash.into());

        // Should always work, since we come from a valid public key.
        let p = CompressedEdwardsY(*pk_bytes).decompress().unwrap();

        // Compute H(C||P)*P.
        DelinearizedPublicKey(s * p)
    }

    pub fn delinearize(public_keys: &[PublicKey]) -> Vec<Self> {
        let mut public_keys = public_keys.to_vec();
        public_keys.sort();
        let h = hash_public_keys(&public_keys);
        public_keys
            .into_iter()
            .map(|pk| DelinearizedPublicKey::new(pk, &h))
            .collect()
    }

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
