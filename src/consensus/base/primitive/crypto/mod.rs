pub mod multisig;

use ed25519_dalek;
use sha2;
use rand::OsRng;

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

impl<'a> From<&'a PrivateKey> for PublicKey {
    fn from(private_key: &'a PrivateKey) -> Self {
        let public_key = ed25519_dalek::PublicKey::from_secret::<sha2::Sha512>(&private_key.key);
        return PublicKey { key: public_key };
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

pub struct KeyPair {
    key_pair: ed25519_dalek::Keypair
}

impl KeyPair {
    pub fn generate() -> Self {
        let mut cspring: OsRng = OsRng::new().unwrap();
        let key_pair = ed25519_dalek::Keypair::generate::<sha2::Sha512>(&mut cspring);
        return KeyPair { key_pair: key_pair };
    }

    pub fn sign(&self, data: &[u8]) -> Signature {
        return signature_create(self, data);
    }
}

impl Signature {
    pub const SIZE: usize = 64;

    #[inline]
    pub fn as_dalek<'a>(&'a self) -> &'a ed25519_dalek::Signature { &self.sig }
}

impl Eq for Signature { }
impl PartialEq for Signature {
    fn eq(&self, other: &Signature) -> bool {
        return self.sig == other.sig;
    }
}

fn signature_create(key_pair: &KeyPair, data: &[u8]) -> Signature {
    let ext_signature = key_pair.key_pair.sign::<sha2::Sha512>(data);
    return Signature { sig: ext_signature };
}

fn signature_verify(signature: &Signature, public_key: &PublicKey, data: &[u8]) -> bool {
    return public_key.as_dalek().verify::<sha2::Sha512>(data, signature.as_dalek());
}
