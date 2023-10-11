use std::fmt;

pub use nimiq_utils::key_rng::{SecureGenerate, SecureRng};
use serde::{Deserialize, Serialize};

pub use self::{
    address::*, errors::*, es256_public_key::*, es256_signature::*, key_pair::*, private_key::*,
    public_key::*, signature::*,
};

#[macro_export]
macro_rules! implement_simple_add_sum_traits {
    ($name: ident, $identity: expr) => {
        impl<'a, 'b> std::ops::Add<&'b $name> for &'a $name {
            type Output = $name;
            fn add(self, other: &'b $name) -> $name {
                $name(self.0 + other.0)
            }
        }
        impl<'b> std::ops::Add<&'b $name> for $name {
            type Output = $name;
            fn add(self, rhs: &'b $name) -> $name {
                &self + rhs
            }
        }

        impl<'a> std::ops::Add<$name> for &'a $name {
            type Output = $name;
            fn add(self, rhs: $name) -> $name {
                self + &rhs
            }
        }

        impl std::ops::Add<$name> for $name {
            type Output = $name;
            fn add(self, rhs: $name) -> $name {
                &self + &rhs
            }
        }

        impl<T> std::iter::Sum<T> for $name
        where
            T: std::borrow::Borrow<$name>,
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
mod es256_signature;
mod key_pair;
mod private_key;
mod public_key;
mod signature;
mod tagged_signing;

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
