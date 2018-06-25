pub mod multisig;

use ed25519_dalek;
use sha2;
use rand::OsRng;
use std::cmp::Ordering;

#[derive(Eq, PartialEq)]
pub struct PublicKey { key: ed25519_dalek::PublicKey }
pub struct PrivateKey { key: ed25519_dalek::SecretKey }
pub struct Signature { sig: ed25519_dalek::Signature }

impl PublicKey {
    pub const SIZE: usize = 32;

    pub fn verify(&self, signature: &Signature, data: &[u8]) -> bool {
        return signature_verify(signature, self, data);
    }

    pub fn sum(public_keys: Vec<PublicKey>) -> Self {
        unimplemented!()
    }

    #[inline]
    pub fn as_bytes<'a>(&'a self) -> &'a [u8; PublicKey::SIZE] { self.key.as_bytes() }

    #[inline]
    pub fn as_dalek<'a>(&'a self) -> &'a ed25519_dalek::PublicKey { &self.key }
}

impl Ord for PublicKey {
    fn cmp(&self, other: &PublicKey) -> Ordering {
        return self.key.as_bytes().cmp(other.key.as_bytes());
    }
}

impl PartialOrd for PublicKey {
    fn partial_cmp(&self, other: &PublicKey) -> Option<Ordering> {
        return Some(self.cmp(other));
    }
}

impl<'a> From<&'a PrivateKey> for PublicKey {
    fn from(private_key: &'a PrivateKey) -> Self {
        let public_key = ed25519_dalek::PublicKey::from_secret::<sha2::Sha512>(private_key.as_dalek());
        return PublicKey { key: public_key };
    }
}

impl<'a> From<&'a [u8; PublicKey::SIZE]> for PublicKey {
    fn from(bytes: &'a [u8; PublicKey::SIZE]) -> Self {
        return PublicKey { key: ed25519_dalek::PublicKey::from_bytes(bytes).unwrap() };
    }
}

impl PrivateKey {
    pub const SIZE: usize = 32;

    pub fn generate() -> Self {
        let mut cspring: OsRng = OsRng::new().unwrap();
        return PrivateKey { key: ed25519_dalek::SecretKey::generate(&mut cspring) };
    }

    #[inline]
    pub fn as_bytes<'a>(&'a self) -> &'a [u8; PrivateKey::SIZE] { self.key.as_bytes() }

    #[inline]
    pub fn as_dalek<'a>(&'a self) -> &'a ed25519_dalek::SecretKey { &self.key }
}

impl<'a> From<&'a [u8; PrivateKey::SIZE]> for PrivateKey {
    fn from(bytes: &'a [u8; PublicKey::SIZE]) -> Self {
        return PrivateKey { key: ed25519_dalek::SecretKey::from_bytes(bytes).unwrap() };
    }
}

pub struct KeyPair {
    key_pair: ed25519_dalek::Keypair
}

impl KeyPair {
    pub fn new(private_key: &PrivateKey, public_key: &PublicKey) -> Self {
        let cloned_priv = ed25519_dalek::SecretKey::from_bytes(private_key.as_bytes()).unwrap();
        let dalek_keypair = ed25519_dalek::Keypair { secret: cloned_priv, public: public_key.as_dalek().clone() };
        return KeyPair { key_pair: dalek_keypair };
    }

    pub fn generate() -> Self {
        let mut cspring: OsRng = OsRng::new().unwrap();
        let key_pair = ed25519_dalek::Keypair::generate::<sha2::Sha512>(&mut cspring);
        return KeyPair { key_pair };
    }

    pub fn sign(&self, data: &[u8]) -> Signature {
        return signature_create(self, data);
    }

    pub fn public(&self) -> PublicKey {
        return PublicKey { key: self.key_pair.public.clone()};
    }

    pub fn private(&self) -> PrivateKey {
        let cloned_key = ed25519_dalek::SecretKey::from_bytes(self.key_pair.secret.as_bytes()).unwrap();
        return PrivateKey { key: cloned_key};
    }
}

impl<'a> From<&'a PrivateKey> for KeyPair {
    fn from(private_key: &PrivateKey) -> Self {
        let public_key = PublicKey::from(private_key);
        let cloned_priv = ed25519_dalek::SecretKey::from_bytes(private_key.as_bytes()).unwrap();
        return KeyPair { key_pair: ed25519_dalek::Keypair { secret: cloned_priv, public: public_key.as_dalek().clone() } };
    }
}

impl Signature {
    pub const SIZE: usize = 64;

    #[inline]
    pub fn to_bytes(&self) -> [u8; Signature::SIZE] { self.sig.to_bytes() }

    #[inline]
    pub fn as_dalek<'a>(&'a self) -> &'a ed25519_dalek::Signature { &self.sig }
}

impl Eq for Signature { }
impl PartialEq for Signature {
    fn eq(&self, other: &Signature) -> bool {
        return self.sig == other.sig;
    }
}

impl<'a> From<&'a [u8; Signature::SIZE]> for Signature {
    fn from(bytes: &'a [u8; Signature::SIZE]) -> Self {
        return Signature { sig: ed25519_dalek::Signature::from_bytes(bytes).unwrap() };
    }
}

fn signature_create(key_pair: &KeyPair, data: &[u8]) -> Signature {
    let ext_signature = key_pair.key_pair.sign::<sha2::Sha512>(data);
    return Signature { sig: ext_signature };
}

fn signature_verify(signature: &Signature, public_key: &PublicKey, data: &[u8]) -> bool {
    return public_key.as_dalek().verify::<sha2::Sha512>(data, signature.as_dalek());
}
