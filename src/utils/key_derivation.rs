use consensus::base::primitive::crypto::{PrivateKey, PublicKey};
use consensus::base::primitive::hash::pbkdf2::*;
use consensus::base::primitive::hash::hmac::*;
use utils::mnemonic::Mnemonic;
use byteorder::{BigEndian, WriteBytesExt};
use beserial::Serialize;
use consensus::base::primitive::hash::Sha512Hash;
use consensus::base::primitive::Address;
use regex::Regex;

const CHAIN_CODE_SIZE: usize = 32;
pub struct ExtendedPrivateKey {
    key: PrivateKey,
    chain_code: [u8; CHAIN_CODE_SIZE],
}

/// This is the byte representation of "ed25519 seed".
const B_CURVE: [u8; 12] = [101, 100, 50, 53, 53, 49, 57, 32, 115, 101, 101, 100];

impl ExtendedPrivateKey {
    pub (in super) fn master_key_from_seed(seed: Vec<u8>) -> Self {
        let hash = compute_hmac_sha512(&B_CURVE, seed.as_slice());
        ExtendedPrivateKey::from(hash)
    }

    pub fn is_valid_path(path: &str) -> bool {
        let re = Regex::new(r"^m(\/[0-9]+')*$").unwrap();
        if !re.is_match(&path) {
            return false;
        }

        // Overflow check.
        path.split("/").skip(1).all(|segment| u32::from_str_radix(segment, 10).is_ok())
    }

    pub fn from_mnemonic(mnemonic: &Mnemonic, password: Option<&str>) -> Result<Self, Pbkdf2Error> {
        Ok(ExtendedPrivateKey::master_key_from_seed(mnemonic.to_seed(password)?))
    }

    pub fn derive(&self, mut index: u32) -> Option<Self> {
        // Only hardened derivation is allowed for ed25519.
        if index < 0x80000000 {
            index = index.checked_add(0x80000000)?;
        }

        let mut data = Vec::<u8>::with_capacity(1 + PrivateKey::SIZE + 4);
        data.write_u8(0).ok()?;
        self.key.serialize(&mut data).ok()?;
        data.write_u32::<BigEndian>(index).ok()?;

        let hash = compute_hmac_sha512(&self.chain_code, data.as_slice());
        Some(ExtendedPrivateKey::from(hash))
    }

    pub fn derive_path(&self, path: &str) -> Option<Self> {
        if !ExtendedPrivateKey::is_valid_path(path) {
            return None;
        }

        let mut derived_key: Option<ExtendedPrivateKey> = None;
        for segment in path.split("/").skip(1) {
            if let Some(key) = derived_key {
                derived_key = Some(key.derive(u32::from_str_radix(segment, 10).unwrap())?);
            } else {
                derived_key = Some(self.derive(u32::from_str_radix(segment, 10).unwrap())?);
            }
        }

        derived_key
    }

    pub fn private_key(self) -> PrivateKey {
        self.key
    }

    pub fn to_address(&self) -> Address {
        let pub_key = PublicKey::from(&self.key);
        Address::from(&pub_key)
    }
}

impl From<Sha512Hash> for ExtendedPrivateKey {
    fn from(hash: Sha512Hash) -> Self {
        let mut private_key: [u8; PrivateKey::SIZE] = Default::default();
        private_key.copy_from_slice(&hash.as_bytes()[..32]);

        let mut chain_code: [u8; CHAIN_CODE_SIZE] = Default::default();
        chain_code.copy_from_slice(&hash.as_bytes()[32..]);

        ExtendedPrivateKey {
            key: PrivateKey::from(private_key),
            chain_code
        }
    }
}

impl From<ExtendedPrivateKey> for PrivateKey {
    fn from(key: ExtendedPrivateKey) -> Self {
        key.private_key()
    }
}

impl<'a> From<&'a ExtendedPrivateKey> for Address {
    fn from(key: &'a ExtendedPrivateKey) -> Self {
        key.to_address()
    }
}
