pub mod multisig;

use ed25519_dalek;
use sha2;
use rand::OsRng;
use std::cmp::Ordering;

#[derive(Eq, PartialEq, Debug, Clone, Copy)]
pub struct PublicKey(ed25519_dalek::PublicKey);
pub struct PrivateKey(ed25519_dalek::SecretKey);
#[derive(Debug)]
pub struct Signature(ed25519_dalek::Signature);
pub struct KeyPair(ed25519_dalek::Keypair);

impl PublicKey {
    pub const SIZE: usize = 32;

    pub fn verify(&self, signature: &Signature, data: &[u8]) -> bool {
        return self.as_dalek().verify::<sha2::Sha512>(data, signature.as_dalek());
    }

    #[inline]
    pub fn as_bytes<'a>(&'a self) -> &'a [u8; PublicKey::SIZE] { self.0.as_bytes() }

    #[inline]
    pub (crate) fn as_dalek<'a>(&'a self) -> &'a ed25519_dalek::PublicKey { &self.0 }
}

impl Ord for PublicKey {
    fn cmp(&self, other: &PublicKey) -> Ordering {
        return self.0.as_bytes().cmp(other.0.as_bytes());
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
        return PublicKey(public_key);
    }
}

impl<'a> From<&'a [u8; PublicKey::SIZE]> for PublicKey {
    fn from(bytes: &'a [u8; PublicKey::SIZE]) -> Self {
        return PublicKey(ed25519_dalek::PublicKey::from_bytes(bytes).unwrap());
    }
}

impl From<[u8; PublicKey::SIZE]> for PublicKey {
    fn from(bytes: [u8; PublicKey::SIZE]) -> Self {
        return PublicKey::from(&bytes);
    }
}

impl PrivateKey {
    pub const SIZE: usize = 32;

    pub fn generate() -> Self {
        let mut cspring: OsRng = OsRng::new().unwrap();
        return PrivateKey(ed25519_dalek::SecretKey::generate(&mut cspring));
    }

    #[inline]
    pub fn as_bytes<'a>(&'a self) -> &'a [u8; PrivateKey::SIZE] { self.0.as_bytes() }

    #[inline]
    pub (crate) fn as_dalek<'a>(&'a self) -> &'a ed25519_dalek::SecretKey { &self.0 }
}

impl<'a> From<&'a [u8; PrivateKey::SIZE]> for PrivateKey {
    fn from(bytes: &'a [u8; PublicKey::SIZE]) -> Self {
        return PrivateKey(ed25519_dalek::SecretKey::from_bytes(bytes).unwrap());
    }
}

impl From<[u8; PrivateKey::SIZE]> for PrivateKey {
    fn from(bytes: [u8; PrivateKey::SIZE]) -> Self {
        return PrivateKey::from(&bytes);
    }
}

impl KeyPair {
    pub fn new(private_key: &PrivateKey, public_key: &PublicKey) -> Self {
        let cloned_priv = ed25519_dalek::SecretKey::from_bytes(private_key.as_bytes()).unwrap();
        let dalek_keypair = ed25519_dalek::Keypair { secret: cloned_priv, public: public_key.as_dalek().clone() };
        return KeyPair(dalek_keypair);
    }

    pub fn generate() -> Self {
        let mut cspring: OsRng = OsRng::new().unwrap();
        let key_pair = ed25519_dalek::Keypair::generate::<sha2::Sha512>(&mut cspring);
        return KeyPair(key_pair);
    }

    pub fn sign(&self, data: &[u8]) -> Signature {
        let ext_signature = self.0.sign::<sha2::Sha512>(data);
        return Signature(ext_signature);
    }

    pub fn public(&self) -> PublicKey {
        return PublicKey(self.0.public.clone());
    }

    pub fn private(&self) -> PrivateKey {
        let cloned_key = ed25519_dalek::SecretKey::from_bytes(self.0.secret.as_bytes()).unwrap();
        return PrivateKey(cloned_key);
    }
}

impl<'a> From<&'a PrivateKey> for KeyPair {
    fn from(private_key: &PrivateKey) -> Self {
        let public_key = PublicKey::from(private_key);
        let cloned_priv = ed25519_dalek::SecretKey::from_bytes(private_key.as_bytes()).unwrap();
        return KeyPair(ed25519_dalek::Keypair { secret: cloned_priv, public: public_key.0 } );
    }
}

impl Signature {
    pub const SIZE: usize = 64;

    #[inline]
    pub fn to_bytes(&self) -> [u8; Signature::SIZE] { self.0.to_bytes() }

    #[inline]
    pub (crate) fn as_dalek<'a>(&'a self) -> &'a ed25519_dalek::Signature { &self.0 }
}

impl Eq for Signature { }
impl PartialEq for Signature {
    fn eq(&self, other: &Signature) -> bool {
        return self.0 == other.0;
    }
}

impl<'a> From<&'a [u8; Signature::SIZE]> for Signature {
    fn from(bytes: &'a [u8; Signature::SIZE]) -> Self {
        return Signature(ed25519_dalek::Signature::from_bytes(bytes).unwrap());
    }
}

impl From<[u8; Signature::SIZE]> for Signature {
    fn from(bytes: [u8; Signature::SIZE]) -> Self {
        return Signature::from(&bytes);
    }
}

fn clone_dalek_sec_key() {

}
