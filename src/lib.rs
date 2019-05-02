#![cfg_attr(not(feature = "std"), no_std)]

use ff::Field;
use group::{CurveAffine, CurveProjective};
use hashmap_core::HashSet;
use pairing::Engine;
use rand::{Rng, SeedableRng};
use rand04_compat::RngExt;
use rand_chacha::ChaChaRng;
use tiny_keccak::sha3_256;

pub mod bls12_381;
#[cfg(feature = "beserial")]
pub mod serialization;

/// Returns a hash of the given message in `G1`.
pub fn hash_g1<E: Engine, M: AsRef<[u8]>>(msg: M) -> E::G1 {
    let digest = sha3_256(msg.as_ref());
    ChaChaRng::from_seed(digest).gen04()
}

pub trait Encoding: Sized {
    type Error;
    type ByteArray;
    const SIZE: usize;

    fn to_bytes(&self) -> Self::ByteArray;
    fn from_bytes(bytes: Self::ByteArray) -> Result<Self, Self::Error>;

    fn from_slice(bytes: &[u8]) -> Result<Self, Self::Error>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Signature<E: Engine> {
    pub(crate) s: E::G1,
}

#[derive(Clone, Copy, Debug, Eq)]
pub struct SecretKey<E: Engine> {
    pub(crate) x: E::Fr,
}

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

    pub fn sign<M: AsRef<[u8]>>(&self, msg: M) -> Signature<E> {
        self.sign_hash(hash_g1::<E, M>(msg))
    }

    pub fn sign_hash<H: Into<E::G1Affine>>(&self, hash: H) -> Signature<E> {
        Signature { s: hash.into().mul(self.x) }
    }
}

#[derive(Clone, Copy, Debug, Eq)]
pub struct PublicKey<E: Engine> {
    pub(crate) p_pub: E::G2,
}

impl<E: Engine> PartialEq for PublicKey<E> {
    fn eq(&self, other: &Self) -> bool {
        self.p_pub.eq(&other.p_pub)
    }
}

impl<E: Engine> PublicKey<E> {
    pub fn from_secret(secret: &SecretKey<E>) -> Self {
        PublicKey {
            p_pub: E::G2Affine::one().mul(secret.x),
        }
    }

    pub fn verify<M: AsRef<[u8]>>(&self, msg: M, signature: &Signature<E>) -> bool {
        self.verify_hash(hash_g1::<E, M>(msg), signature)
    }

    pub fn verify_hash<H: Into<E::G1Affine>>(&self, hash: H, signature: &Signature<E>) -> bool {
        let lhs = E::pairing(signature.s, E::G2Affine::one());
        let rhs = E::pairing(hash.into(), self.p_pub);
        lhs == rhs
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Keypair<E: Engine> {
    pub secret: SecretKey<E>,
    pub public: PublicKey<E>,
}

impl<E: Engine> Keypair<E> {
    pub fn generate<R: Rng>(csprng: &mut R) -> Self {
        let secret = SecretKey::generate(csprng);
        let public = PublicKey::from_secret(&secret);
        Keypair { secret, public }
    }

    pub fn sign<M: AsRef<[u8]>>(&self, msg: M) -> Signature<E> {
        self.secret.sign(msg)
    }

    pub fn verify<M: AsRef<[u8]>>(&self, msg: M, signature: &Signature<E>) -> bool {
        self.public.verify(msg, signature)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
    pub fn verify<M: AsRef<[u8]>>(&self, msg: M, signature: &AggregateSignature<E>) -> bool {
        self.0.verify(msg, &signature.0)
    }

    pub fn verify_hash<H: Into<E::G1Affine>>(&self, hash: H, signature: &AggregateSignature<E>) -> bool {
        self.0.verify_hash(hash, &signature.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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

    pub fn verify<M: AsRef<[u8]>>(&self, public_keys: &[PublicKey<E>], msgs: &[M]) -> bool {
        // Number of messages must coincide with number of public keys.
        if public_keys.len() != msgs.len() {
            return false;
        }
        // Messages must be distinct.
        let messages: HashSet<&[u8]> = msgs.iter().map(|msg| msg.as_ref()).collect();
        if messages.len() != msgs.len() {
            return false;
        }
        // Check pairings.
        let lhs = E::pairing(self.0.s, E::G2Affine::one());
        let mut rhs = E::Fqk::one();
        for i in 0..public_keys.len() {
            let h = hash_g1::<E, &[u8]>(msgs[i].as_ref());
            rhs.mul_assign(&E::pairing(h, public_keys[i].p_pub));
        }
        lhs == rhs
    }
}

#[cfg(test)]
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
}
