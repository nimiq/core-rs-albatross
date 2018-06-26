pub mod multisig;

use std::cmp::Ordering;
use std::io;

use ed25519_dalek;
use rand::OsRng;
use beserial::{Serialize,Deserialize,ReadBytesExt,WriteBytesExt};
use sha2;

#[derive(Eq, PartialEq, Debug, Clone, Copy)]
pub struct PublicKey(ed25519_dalek::PublicKey);
pub struct PrivateKey(ed25519_dalek::SecretKey);
#[derive(Debug)]
pub struct Signature(ed25519_dalek::Signature);
pub struct KeyPair {
    pub public: PublicKey,
    pub private: PrivateKey
}

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

impl Deserialize for PublicKey {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> io::Result<Self> {
        let mut buf = [0u8; PublicKey::SIZE];
        reader.read_exact(&mut buf)?;
        return Ok(PublicKey::from(&buf));
    }
}

impl Serialize for PublicKey {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> io::Result<usize> {
        writer.write(self.as_bytes())?;
        return Ok(self.serialized_size());
    }

    fn serialized_size(&self) -> usize {
        return PublicKey::SIZE;
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

impl Clone for PrivateKey {
    fn clone(&self) -> Self {
        let cloned_dalek = ed25519_dalek::SecretKey::from_bytes(self.0.as_bytes()).unwrap();
        return PrivateKey(cloned_dalek);
    }
}

impl KeyPair {
    pub fn generate() -> Self {
        let mut cspring: OsRng = OsRng::new().unwrap();
        let key_pair = ed25519_dalek::Keypair::generate::<sha2::Sha512>(&mut cspring);
        let priv_key = PrivateKey(key_pair.secret);
        let pub_key = PublicKey(key_pair.public);
        return KeyPair { private: priv_key, public: pub_key };
    }

    pub fn sign(&self, data: &[u8]) -> Signature {
        let ext_signature = self.private.0.expand::<sha2::Sha512>().sign::<sha2::Sha512>(data, &self.public.0);
        return Signature(ext_signature);
    }
}

impl From<PrivateKey> for KeyPair {
    fn from(private_key: PrivateKey) -> Self {
        return KeyPair { public: PublicKey::from(&private_key), private: private_key };
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

impl Deserialize for Signature {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> io::Result<Self> {
        let mut buf = [0u8; Signature::SIZE];
        reader.read_exact(&mut buf)?;
        return Ok(Signature::from(&buf));
    }
}

impl Serialize for Signature {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> io::Result<usize> {
        writer.write(&self.to_bytes())?;
        return Ok(self.serialized_size());
    }

    fn serialized_size(&self) -> usize {
        return Signature::SIZE;
    }
}
