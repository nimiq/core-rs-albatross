use std::fmt;

pub use nimiq_utils::key_rng::{SecureGenerate, SecureRng};
use serde::{Deserialize, Serialize};

pub use self::{
    address::*, errors::*, es256_public_key::*, key_pair::*, private_key::*, public_key::*,
    signature::*,
};

#[macro_export]
macro_rules! implement_simple_add_sum_traits {
    ($name: ident, $identity: expr) => {
        impl<'a, 'b> Add<&'b $name> for &'a $name {
            type Output = $name;
            fn add(self, other: &'b $name) -> $name {
                $name(self.0 + other.0)
            }
        }
        impl<'b> Add<&'b $name> for $name {
            type Output = $name;
            fn add(self, rhs: &'b $name) -> $name {
                &self + rhs
            }
        }

        impl<'a> Add<$name> for &'a $name {
            type Output = $name;
            fn add(self, rhs: $name) -> $name {
                self + &rhs
            }
        }

        impl Add<$name> for $name {
            type Output = $name;
            fn add(self, rhs: $name) -> $name {
                &self + &rhs
            }
        }

        impl<T> Sum<T> for $name
        where
            T: Borrow<$name>,
        {
            fn sum<I>(iter: I) -> Self
            where
                I: Iterator<Item = T>,
            {
                $name(iter.fold($identity, |acc, item| acc + item.borrow().0))
            }
        }
    };
}

pub mod multisig;

mod address;
mod errors;
mod es256_public_key;
mod key_pair;
mod private_key;
mod public_key;
mod signature;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Deserialize, Serialize)]
#[cfg_attr(feature = "serde-derive", derive(nimiq_hash_derive::SerializeContent))]
pub enum PublicKey {
    EdDSA(EdDSAPublicKey),
    ECDSA(ES256PublicKey),
}

impl PublicKey {
    pub fn as_bytes(&self) -> &[u8] {
        match self {
            PublicKey::EdDSA(key) => key.as_bytes(),
            PublicKey::ECDSA(key) => key.as_bytes(),
        }
    }

    pub fn verify(&self, signature: &Signature, data: &[u8]) -> bool {
        match self {
            PublicKey::EdDSA(key) => key.verify(signature, data),
            PublicKey::ECDSA(key) => key.verify(signature, data),
        }
    }

    pub fn to_hex(&self) -> String {
        match self {
            PublicKey::EdDSA(key) => key.to_hex(),
            PublicKey::ECDSA(key) => key.to_hex(),
        }
    }
}

impl Default for PublicKey {
    fn default() -> Self {
        PublicKey::EdDSA(EdDSAPublicKey::default())
    }
}

impl fmt::Display for PublicKey {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        f.write_str(&self.to_hex())
    }
}

impl From<EdDSAPublicKey> for PublicKey {
    fn from(key: EdDSAPublicKey) -> Self {
        PublicKey::EdDSA(key)
    }
}

impl From<ES256PublicKey> for PublicKey {
    fn from(key: ES256PublicKey) -> Self {
        PublicKey::ECDSA(key)
    }
}
