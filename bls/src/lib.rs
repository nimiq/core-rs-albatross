#![cfg_attr(not(feature = "std"), no_std)]

extern crate nimiq_hash as hash;
extern crate hex;

use ff::Field;
use group::{CurveAffine, CurveProjective};
use hashmap_core::HashSet;
use pairing::Engine;
use rand::{Rng, SeedableRng};
use rand04_compat::RngExt;
use rand_chacha::ChaChaRng;

use hash::{Hash, Blake2bHash};

pub mod bls12_381;
#[cfg(feature = "beserial")]
pub mod serialization;

/// Hash used for signatures
pub type SigHash = Blake2bHash;

/// Map hash to point in G1
pub(crate) fn hash_to_g1<E: Engine>(h: SigHash) -> E::G1 {
    ChaChaRng::from_seed(h.into()).gen04()
}

#[derive(Clone, Copy)]
pub struct Signature<E: Engine> {
    pub(crate) s: E::G1,
}

impl<E: Engine> Eq for Signature<E>{}
impl<E: Engine> PartialEq for Signature<E> {
    fn eq(&self, other: &Self) -> bool {
        self.s.eq(&other.s)
    }
}

#[derive(Clone, Copy)]
pub struct SecretKey<E: Engine> {
    pub(crate) x: E::Fr,
}

impl<E: Engine> Eq for SecretKey<E>{}
impl<E: Engine> PartialEq for SecretKey<E> {
    fn eq(&self, other: &Self) -> bool {
        self.x.eq(&other.x)
    }
}

impl<E: Engine> SecretKey<E> {
    pub fn generate<R: Rng>(csprng: &mut R) -> Self {
        SecretKey {
            x: csprng.gen04(),
        }
    }

    pub fn sign<M: Hash>(&self, msg: &M) -> Signature<E> {
        self.sign_hash(msg.hash())
    }

    pub fn sign_hash(&self, hash: SigHash) -> Signature<E> {
        self.sign_g1(hash_to_g1::<E>(hash))
    }

    fn sign_g1<H: Into<E::G1Affine>>(&self, h: H) -> Signature<E> {
        Signature { s: h.into().mul(self.x) }
    }
}

#[derive(Clone, Copy)]
pub struct PublicKey<E: Engine> {
    pub(crate) p_pub: E::G2,
}

impl<E: Engine> PartialEq for PublicKey<E> {
    fn eq(&self, other: &Self) -> bool {
        self.p_pub.eq(&other.p_pub)
    }
}

impl<E: Engine> Eq for PublicKey<E> {}

impl<E: Engine> PublicKey<E> {
    pub fn from_secret(secret: &SecretKey<E>) -> Self {
        PublicKey {
            p_pub: E::G2Affine::one().mul(secret.x),
        }
    }

    pub fn verify<M: Hash>(&self, msg: &M, signature: &Signature<E>) -> bool {
        self.verify_hash(msg.hash(), signature)
    }

    pub fn verify_hash(&self, hash: SigHash, signature: &Signature<E>) -> bool {
        self.verify_g1(hash_to_g1::<E>(hash), signature)
    }

    fn verify_g1<H: Into<E::G1Affine>>(&self, h: H, signature: &Signature<E>) -> bool {
        let lhs = E::pairing(signature.s, E::G2Affine::one());
        let rhs = E::pairing(h.into(), self.p_pub);
        lhs == rhs
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct KeyPair<E: Engine> {
    pub secret: SecretKey<E>,
    pub public: PublicKey<E>,
}

impl<E: Engine> KeyPair<E> {
    pub fn generate<R: Rng>(csprng: &mut R) -> Self {
        let secret = SecretKey::generate(csprng);
        KeyPair::from(secret)
    }

    pub fn from_secret(secret: &SecretKey<E>) -> Self {
        KeyPair::from(secret.clone())
    }

    pub fn sign<M: Hash>(&self, msg: &M) -> Signature<E> {
        self.secret.sign::<M>(msg)
    }

    pub fn sign_hash(&self, hash: SigHash) -> Signature<E> {
        self.secret.sign_hash(hash)
    }

    pub fn verify<M: Hash>(&self, msg: &M, signature: &Signature<E>) -> bool {
        self.public.verify::<M>(msg, signature)
    }

    pub fn verify_hash<H>(&self, hash: SigHash, signature: &Signature<E>) -> bool {
        self.public.verify_hash(hash, signature)
    }
}

impl<E: Engine> From<SecretKey<E>> for KeyPair<E> {
    fn from(secret: SecretKey<E>) -> Self {
        let public = PublicKey::from_secret(&secret);
        KeyPair {
            secret,
            public,
        }
    }
}

#[derive(Clone, Copy)]
pub struct AggregatePublicKey<E: Engine>(pub(crate) PublicKey<E>);

impl<E: Engine> AggregatePublicKey<E> {
    pub fn new() -> Self {
        AggregatePublicKey(PublicKey { p_pub: E::G2::zero() })
    }

    /// When using this method, it is essential that there exist proofs of knowledge
    /// of the secret key for each public key.
    /// Otherwise, an adversary can submit a public key to cancel out other public keys.
    pub fn from_public_keys(keys: &[PublicKey<E>]) -> Self {
        let mut pkey = Self::new();
        for key in keys {
            pkey.aggregate(key);
        }
        pkey
    }

    /// When using this method, it is essential that there exist proofs of knowledge
    /// of the secret key for each public key.
    /// Otherwise, an adversary can submit a public key to cancel out other public keys.
    pub fn aggregate(&mut self, key: &PublicKey<E>) {
        self.0.p_pub.add_assign(&key.p_pub);
    }

    pub fn merge_into(&mut self, other: &Self) {
        self.0.p_pub.add_assign(&other.0.p_pub);
    }


    /// Verify an aggregate signature over the same message.
    pub fn verify<M: Hash>(&self, msg: &M, signature: &AggregateSignature<E>) -> bool {
        self.0.verify::<M>(msg, &signature.0)
    }

    pub fn verify_hash(&self, hash: SigHash, signature: &AggregateSignature<E>) -> bool {
        self.0.verify_hash(hash, &signature.0)
    }
}

impl<E: Engine> Eq for AggregatePublicKey<E> {}
impl<E: Engine> PartialEq for AggregatePublicKey<E> {
    fn eq(&self, other: &Self) -> bool {
        self.0.eq(&other.0)
    }
}

#[derive(Clone, Copy)]
pub struct AggregateSignature<E: Engine>(pub(crate) Signature<E>);

impl<E: Engine> AggregateSignature<E> {
    pub fn new() -> Self {
        AggregateSignature(Signature { s: E::G1::zero() })
    }

    pub fn from_signatures(sigs: &[Signature<E>]) -> Self {
        let mut s = Self::new();
        for sig in sigs {
            s.aggregate(sig);
        }
        s
    }

    pub fn aggregate(&mut self, sig: &Signature<E>) {
        self.0.s.add_assign(&sig.s);
    }

    pub fn merge_into(&mut self, other: &Self) {
        self.0.s.add_assign(&other.0.s);
    }

    pub fn verify<M: Hash>(&self, public_keys: &[PublicKey<E>], msgs: &[M]) -> bool {
        // Number of messages must coincide with number of public keys.
        if public_keys.len() != msgs.len() {
            panic!("Different amount of messages and public keys");
        }

        // compute hashes
        let mut hashes: Vec<SigHash> = msgs.iter().rev().map(|msg| msg.hash::<SigHash>()).collect();

        // check that hashes are distinct
        let distinct_hashes: HashSet<&SigHash> = hashes.iter().collect();
        if distinct_hashes.len() != hashes.len() {
            panic!("Messages are not distinct");
        }

        // Check pairings.
        let lhs = E::pairing(self.0.s, E::G2Affine::one());
        let mut rhs = E::Fqk::one();
        for i in 0..public_keys.len() {
            // garantueed to be available, since we check that there are as many messages/hashes
            // as public_keys.
            let h = hashes.pop().unwrap();
            rhs.mul_assign(&E::pairing(hash_to_g1::<E>(h), public_keys[i].p_pub));
        }
        lhs == rhs
    }
}

impl<E: Engine> Eq for AggregateSignature<E> {}
impl<E: Engine> PartialEq for AggregateSignature<E> {
    fn eq(&self, other: &Self) -> bool {
        self.0.eq(&other.0)
    }
}

/*#[cfg(test)]
mod tests {
    use std::vec::Vec;

    use pairing::bls12_381::Bls12;
    use rand::SeedableRng;
    use rand_xorshift::XorShiftRng;

    use super::*;

    #[test]
    fn sign_verify() {
        let mut rng = XorShiftRng::from_seed([0x44, 0x6d, 0x4f, 0xbc, 0x6c, 0x27, 0x2f, 0xd6, 0xd0, 0xaf, 0x63, 0xb9, 0x3d, 0x86, 0x55, 0x54]);

        for i in 0..500 {
            let keypair = Keypair::<Bls12>::generate(&mut rng);
            let message = format!("Message {}", i);
            let sig = keypair.sign(&message.as_bytes());
            assert_eq!(keypair.verify(&message.as_bytes(), &sig), true);
        }
    }

    #[test]
    fn aggregate_signatures() {
        let mut rng = XorShiftRng::from_seed([0x44, 0x6d, 0x4f, 0xbc, 0x6c, 0x27, 0x2f, 0xd6, 0xd0, 0xaf, 0x63, 0xb9, 0x3d, 0x86, 0x55, 0x54]);

        let mut public_keys = Vec::with_capacity(1000);
        let mut messages = Vec::with_capacity(1000);
        let mut signatures = Vec::with_capacity(1000);
        for i in 0..500 {
            let keypair = Keypair::<Bls12>::generate(&mut rng);
            let message = format!("Message {}", i);
            let signature = keypair.sign(&message);
            public_keys.push(keypair.public);
            messages.push(message);
            signatures.push(signature);

            // Only test near the beginning and the end, to reduce test runtime
            if i < 10 || i > 495 {
                let asig = AggregateSignature::from_signatures(&signatures);
                assert_eq!(
                    asig.verify(&public_keys, &messages),
                    true
                );
            }
        }
    }

    #[test]
    fn aggregate_signatures_duplicated_messages() {
        let mut rng = XorShiftRng::from_seed([0x44, 0x6d, 0x4f, 0xbc, 0x6c, 0x27, 0x2f, 0xd6, 0xd0, 0xaf, 0x63, 0xb9, 0x3d, 0x86, 0x55, 0x54]);

        let mut public_keys = Vec::new();
        let mut messages = Vec::new();
        let mut asig = AggregateSignature::new();

        // Create the first signature
        let keypair = Keypair::<Bls12>::generate(&mut rng);
        let message = "First message";
        let signature = keypair.sign(&message.as_bytes());
        public_keys.push(keypair.public);
        messages.push(message);
        asig.aggregate(&signature);

        // The first "aggregate" signature should pass
        assert_eq!(
            asig.verify(&public_keys, &messages),
            true
        );

        // Create the second signature
        let keypair = Keypair::<Bls12>::generate(&mut rng);
        let message = "Second message";
        let signature = keypair.sign(&message.as_bytes());
        public_keys.push(keypair.public);
        messages.push(message);
        asig.aggregate(&signature);

        // The second (now-)aggregate signature should pass
        assert_eq!(
            asig.verify(&public_keys, &messages),
            true
        );

        // Create the third signature, reusing the second message
        let keypair = Keypair::<Bls12>::generate(&mut rng);
        let signature = keypair.sign(&message.as_bytes());
        public_keys.push(keypair.public);
        messages.push(message);
        asig.aggregate(&signature);

        // The third aggregate signature should fail
        assert_eq!(
            asig.verify(&public_keys, &messages),
            false
        );
    }

    #[test]
    fn aggregate_signatures_same_messages() {
        let mut rng = XorShiftRng::from_seed([0x44, 0x6d, 0x4f, 0xbc, 0x6c, 0x27, 0x2f, 0xd6, 0xd0, 0xaf, 0x63, 0xb9, 0x3d, 0x86, 0x55, 0x54]);

        let mut public_keys = Vec::with_capacity(1000);
        let message = "Same message";
        let mut signatures = Vec::with_capacity(1000);
        for _ in 0..500 {
            let keypair = Keypair::<Bls12>::generate(&mut rng);
            let signature = keypair.sign(&message);
            public_keys.push(keypair.public);
            signatures.push(signature);
        }

        let akey = AggregatePublicKey::from_public_keys(&public_keys);
        let asig = AggregateSignature::from_signatures(&signatures);

        assert_eq!(
            akey.verify(message, &asig),
            true
        );
    }
}*/
