use std::borrow::Cow;

use byteorder::{BigEndian, WriteBytesExt};
use nimiq_hash::{hmac::*, sha512::Sha512Hash};
use nimiq_keys::{Address, EdDSAPublicKey, PrivateKey};
use nimiq_serde::Serialize;
use regex::Regex;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExtendedPrivateKey {
    key: PrivateKey,
    chain_code: [u8; ExtendedPrivateKey::CHAIN_CODE_SIZE],
}

/// This is the byte representation of "ed25519 seed".
const B_CURVE: [u8; 12] = [101, 100, 50, 53, 53, 49, 57, 32, 115, 101, 101, 100];

impl ExtendedPrivateKey {
    pub const CHAIN_CODE_SIZE: usize = 32;

    /// Returns the corresponding master extended private key for a seed.
    pub fn from_seed(seed: Vec<u8>) -> Self {
        let hash = compute_hmac_sha512(&B_CURVE, seed.as_slice());
        ExtendedPrivateKey::from(hash)
    }

    /// Checks whether a string is a valid derivation path.
    pub fn is_valid_path(path: &str) -> bool {
        let re = Regex::new(r"^m(/[0-9]+')*$").unwrap();
        if !re.is_match(path) {
            return false;
        }

        // Overflow check.
        path.split('/')
            .skip(1)
            .all(|segment| segment.trim_end_matches('\'').parse::<u32>().is_ok())
    }

    /// Returns the derived extended private key at a given index.
    pub fn derive(&self, mut index: u32) -> Option<Self> {
        // Only hardened derivation is allowed for ed25519.
        if index < 0x8000_0000 {
            index = index.checked_add(0x8000_0000)?;
        }

        let mut data = Vec::<u8>::with_capacity(1 + PrivateKey::SIZE + 4);
        data.write_u8(0).ok()?;
        self.key.serialize_to_writer(&mut data).ok()?;
        data.write_u32::<BigEndian>(index).ok()?;

        let hash = compute_hmac_sha512(&self.chain_code, data.as_slice());
        Some(ExtendedPrivateKey::from(hash))
    }

    /// Derives a key by path.
    pub fn derive_path(&self, path: &str) -> Option<Self> {
        if !ExtendedPrivateKey::is_valid_path(path) {
            return None;
        }

        let mut derived_key: Cow<ExtendedPrivateKey> = Cow::Borrowed(self);
        for segment in path.split('/').skip(1) {
            derived_key = Cow::Owned(
                derived_key.derive(segment.trim_end_matches('\'').parse::<u32>().unwrap())?,
            );
        }

        Some(derived_key.into_owned())
    }

    /// Returns a reference to the chain code slice.
    pub fn get_chain_code(&self) -> &[u8] {
        &self.chain_code
    }

    /// Converts the ExtendedPrivateKey format into a normal PrivateKey, loosing the ability for derivation.
    pub fn into_private_key(self) -> PrivateKey {
        self.key
    }

    /// Returns the public key for this private key.
    pub fn to_public_key(&self) -> EdDSAPublicKey {
        EdDSAPublicKey::from(&self.key)
    }

    /// Returns the public address for this private key.
    pub fn to_address(&self) -> Address {
        Address::from(&self.to_public_key())
    }
}

impl From<Sha512Hash> for ExtendedPrivateKey {
    fn from(hash: Sha512Hash) -> Self {
        let mut private_key: [u8; PrivateKey::SIZE] = Default::default();
        private_key.copy_from_slice(&hash.as_bytes()[..32]);

        let mut chain_code: [u8; ExtendedPrivateKey::CHAIN_CODE_SIZE] = Default::default();
        chain_code.copy_from_slice(&hash.as_bytes()[32..]);

        ExtendedPrivateKey {
            key: PrivateKey::from(private_key),
            chain_code,
        }
    }
}

impl From<ExtendedPrivateKey> for PrivateKey {
    fn from(key: ExtendedPrivateKey) -> Self {
        key.into_private_key()
    }
}

impl<'a> From<&'a ExtendedPrivateKey> for Address {
    fn from(key: &'a ExtendedPrivateKey) -> Self {
        key.to_address()
    }
}

impl<'a> From<&'a ExtendedPrivateKey> for EdDSAPublicKey {
    fn from(key: &'a ExtendedPrivateKey) -> Self {
        key.to_public_key()
    }
}
