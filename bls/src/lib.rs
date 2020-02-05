#[macro_use]
extern crate failure;
extern crate hex;
extern crate nimiq_hash as hash;
extern crate nimiq_utils as utils;

use algebra::{
    curves::{
        bls12_377::{Bls12_377, G1Affine, G1Projective, G2Affine, G2Projective},
        AffineCurve, PairingEngine, ProjectiveCurve,
    },
    fields::{
        bls12_377::{Fq, Fr},
        Field, FpParameters,
    },
    CanonicalSerialize,
};

use hashbrown::HashSet;
use rand::SeedableRng;
use rand_chacha::ChaChaRng;

use hash::{Blake2bHash, Hash};
use utils::key_rng::{CryptoRng, Rng};
pub use utils::key_rng::{SecureGenerate, SecureRng};

// pub mod bls12_381;
// #[cfg(feature = "beserial")]
// pub mod serialization;

/// Hash used for signatures
pub type SigHash = Blake2bHash;

/// Map hash to point in G1Projective
// pub(crate) fn hash_to_g1(h: SigHash) -> G1Projective {
//     G1Projective::random(&mut ChaChaRng::from_seed(h.into()))
// }

#[derive(Clone, Copy)]
pub struct Signature {
    // The projective form is the longer one, with 3 coordinates. It is meant only for quick calculation.
    // We can't use the affine form since the Algebra library doesn't support arithmetic with it.
    pub(crate) signature: G1Projective,
}

impl Eq for Signature {}

impl PartialEq for Signature {
    fn eq(&self, other: &Self) -> bool {
        self.signature.eq(&other.signature)
    }
}

#[derive(Clone, Copy)]
pub struct SecretKey {
    pub(crate) secret_key: Fr,
}

impl Eq for SecretKey {}

impl PartialEq for SecretKey {
    fn eq(&self, other: &Self) -> bool {
        self.secret_key.eq(&other.secret_key)
    }
}

impl SecretKey {
    // #[cfg(test)]
    // fn generate_predictable<R: Rng>(rng: &mut R) -> Self {
    //     SecretKey {
    //         x: E::Fr::random(rng),
    //     }
    // }

    pub fn sign<M: Hash>(&self, msg: &M) -> Signature {
        self.sign_hash(msg.hash())
    }

    pub fn sign_hash(&self, hash: SigHash) -> Signature {
        self.sign_g1(hash_to_g1(hash))
    }

    fn sign_g1(&self, hash_curve: G1Projective) -> Signature {
        Signature {
            signature: &self * hash_curve,
        }
    }
}

impl SecureGenerate for SecretKey {
    fn generate<R: Rng + CryptoRng>(rng: &mut R) -> Self {
        SecretKey {
            secret_key: Fr::random(rng),
        }
    }
}

#[derive(Clone, Copy)]
pub struct PublicKey {
    pub(crate) public_key: G2Projective,
}

impl Eq for PublicKey {}

impl PartialEq for PublicKey {
    fn eq(&self, other: &Self) -> bool {
        self.public_key.eq(&other.public_key)
    }
}

impl PublicKey {
    pub fn from_secret(x: &SecretKey) -> Self {
        PublicKey {
            public_key: G2Projective::prime_subgroup_generator() * x.secret_key,
        }
    }

    pub fn verify<M: Hash>(&self, msg: &M, signature: &Signature) -> bool {
        self.verify_hash(msg.hash(), signature)
    }

    pub fn verify_hash(&self, hash: SigHash, signature: &Signature) -> bool {
        self.verify_g1(hash_to_g1(hash), signature)
    }

    fn verify_g1(&self, hash_curve: G1Projective, signature: &Signature) -> bool {
        let lhs = Bls12_377::pairing(
            signature.signature,
            G2Projective::prime_subgroup_generator(),
        );
        let rhs = Bls12_377::pairing(hash_curve, self.public_key);
        lhs == rhs
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct KeyPair {
    pub secret_key: SecretKey,
    pub public_key: PublicKey,
}

impl KeyPair {
    // #[cfg(test)]
    // fn generate_predictable<R: Rng>(rng: &mut R) -> Self {
    //     let secret = SecretKey::generate_predictable(rng);
    //     KeyPair::from(secret)
    // }

    pub fn from_secret(secret: &SecretKey) -> Self {
        KeyPair::from(secret.clone())
    }

    pub fn sign<M: Hash>(&self, msg: &M) -> Signature {
        self.secret_key.sign::<M>(msg)
    }

    pub fn sign_hash(&self, hash: SigHash) -> Signature {
        self.secret_key.sign_hash(hash)
    }

    pub fn verify<M: Hash>(&self, msg: &M, signature: &Signature) -> bool {
        self.public_key.verify::<M>(msg, signature)
    }

    pub fn verify_hash(&self, hash: SigHash, signature: &Signature) -> bool {
        self.public_key.verify_hash(hash, signature)
    }
}

impl SecureGenerate for KeyPair {
    fn generate<R: Rng + CryptoRng>(rng: &mut R) -> Self {
        let secret = SecretKey::generate(rng);
        KeyPair::from(secret)
    }
}

impl From<SecretKey> for KeyPair {
    fn from(secret: SecretKey) -> Self {
        let public = PublicKey::from_secret(&secret);
        KeyPair { secret, public }
    }
}

#[derive(Clone, Copy)]
pub struct AggregatePublicKey(pub(crate) PublicKey);

impl AggregatePublicKey {
    pub fn new() -> Self {
        AggregatePublicKey(PublicKey {
            public_key: E::G2::zero(),
        })
    }

    /// When using this method, it is essential that there exist proofs of knowledge
    /// of the secret key for each public key.
    /// Otherwise, an adversary can submit a public key to cancel out other public keys.
    pub fn from_public_keys(keys: &[PublicKey]) -> Self {
        let mut pkey = Self::new();
        for key in keys {
            pkey.aggregate(key);
        }
        pkey
    }

    /// When using this method, it is essential that there exist proofs of knowledge
    /// of the secret key for each public key.
    /// Otherwise, an adversary can submit a public key to cancel out other public keys.
    pub fn aggregate(&mut self, key: &PublicKey) {
        self.0.public_key.add_assign(&key.public_key);
    }

    pub fn merge_into(&mut self, other: &Self) {
        self.0.public_key.add_assign(&other.0.public_key);
    }

    /// Verify an aggregate signature over the same message.
    pub fn verify<M: Hash>(&self, msg: &M, signature: &AggregateSignature) -> bool {
        self.0.verify::<M>(msg, &signature.0)
    }

    pub fn verify_hash(&self, hash: SigHash, signature: &AggregateSignature) -> bool {
        self.0.verify_hash(hash, &signature.0)
    }
}

impl Eq for AggregatePublicKey {}

impl PartialEq for AggregatePublicKey {
    fn eq(&self, other: &Self) -> bool {
        self.0.eq(&other.0)
    }
}

impl Default for AggregatePublicKey {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy)]
pub struct AggregateSignature(pub Signature);

impl AggregateSignature {
    pub fn new() -> Self {
        AggregateSignature(Signature { s: E::G1::zero() })
    }

    pub fn from_signatures(sigs: &[Signature]) -> Self {
        let mut s = Self::new();
        for sig in sigs {
            s.aggregate(sig);
        }
        s
    }

    pub fn aggregate(&mut self, sig: &Signature) {
        self.0.s.add_assign(&sig.s);
    }

    pub fn merge_into(&mut self, other: &Self) {
        self.0.s.add_assign(&other.0.s);
    }

    pub fn verify<M: Hash>(&self, public_keys: &[PublicKey], msgs: &[M]) -> bool {
        // Number of messages must coincide with number of public keys.
        if public_keys.len() != msgs.len() {
            panic!("Different amount of messages and public keys");
        }

        // compute hashes
        let mut hashes: Vec<SigHash> = msgs.iter().rev().map(|msg| msg.hash::<SigHash>()).collect();

        // check that hashes are distinct
        // TODO: scoping currently required for borrow checker
        {
            let distinct_hashes: HashSet<&SigHash> = hashes.iter().collect();
            if distinct_hashes.len() != hashes.len() {
                panic!("Messages are not distinct");
            }
        }

        // Check pairings.
        let lhs = E::pairing(self.0.s, E::G2Affine::one());
        let mut rhs = E::Fqk::one();
        for public_key in public_keys {
            // garantueed to be available, since we check that there are as many messages/hashes
            // as public_keys.
            let h = hashes.pop().unwrap();
            rhs.mul_assign(&E::pairing(hash_to_g1::<E>(h), public_key.public_key));
        }
        lhs == rhs
    }
}

impl Eq for AggregateSignature {}

impl PartialEq for AggregateSignature {
    fn eq(&self, other: &Self) -> bool {
        self.0.eq(&other.0)
    }
}

impl Default for AggregateSignature {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use rand_xorshift::XorShiftRng;
    use std::vec::Vec;

    use super::*;

    #[test]
    fn sign_verify() {
        let mut rng = XorShiftRng::from_seed([
            0x44, 0x6d, 0x4f, 0xbc, 0x6c, 0x27, 0x2f, 0xd6, 0xd0, 0xaf, 0x63, 0xb9, 0x3d, 0x86,
            0x55, 0x54,
        ]);

        for i in 0..500 {
            let keypair = KeyPair::<Bls12>::generate_predictable(&mut rng);
            let message = format!("Message {}", i);
            let sig = keypair.sign(&message);
            assert_eq!(keypair.verify(&message.as_bytes(), &sig), true);
        }
    }

    #[test]
    fn aggregate_signatures() {
        let mut rng = XorShiftRng::from_seed([
            0x44, 0x6d, 0x4f, 0xbc, 0x6c, 0x27, 0x2f, 0xd6, 0xd0, 0xaf, 0x63, 0xb9, 0x3d, 0x86,
            0x55, 0x54,
        ]);

        let mut public_keys = Vec::with_capacity(1000);
        let mut messages = Vec::with_capacity(1000);
        let mut signatures = Vec::with_capacity(1000);
        for i in 0..500 {
            let keypair = KeyPair::<Bls12>::generate_predictable(&mut rng);
            let message = format!("Message {}", i);
            let signature = keypair.sign(&message);
            public_keys.push(keypair.public);
            messages.push(message);
            signatures.push(signature);

            // Only test near the beginning and the end, to reduce test runtime
            if i < 10 || i > 495 {
                let asig = AggregateSignature::from_signatures(&signatures);
                assert_eq!(asig.verify(&public_keys, &messages), true);
            }
        }
    }

    #[test]
    fn aggregate_signatures_same_messages() {
        let mut rng = XorShiftRng::from_seed([
            0x44, 0x6d, 0x4f, 0xbc, 0x6c, 0x27, 0x2f, 0xd6, 0xd0, 0xaf, 0x63, 0xb9, 0x3d, 0x86,
            0x55, 0x54,
        ]);

        let mut public_keys = Vec::with_capacity(1000);
        let message = "Same message";
        let mut signatures = Vec::with_capacity(1000);
        for _ in 0..500 {
            let keypair = KeyPair::<Bls12>::generate_predictable(&mut rng);
            let signature = keypair.sign(&message);
            public_keys.push(keypair.public);
            signatures.push(signature);
        }

        let akey = AggregatePublicKey::from_public_keys(&public_keys);
        let asig = AggregateSignature::from_signatures(&signatures);

        assert_eq!(akey.verify(&message, &asig), true);
    }
}
