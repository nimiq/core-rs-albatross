use ed25519_dalek;
use rand::OsRng;

use consensus::base::primitive::crypto::{PublicKey};

pub struct PrivateKey(pub(in super) ed25519_dalek::SecretKey);

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
