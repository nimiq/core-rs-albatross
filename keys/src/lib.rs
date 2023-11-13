use nimiq_serde::{Deserialize, Serialize};
pub use nimiq_utils::key_rng::{SecureGenerate, SecureRng};

pub use self::{
    address::*, errors::*, es256_public_key::*, es256_signature::*, key_pair::*, private_key::*,
    public_key::*, signature::*,
};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum PublicKey {
    Ed25519(public_key::EdDSAPublicKey),
    ES256(es256_public_key::ES256PublicKey),
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum SignatureEnum {
    Ed25519(signature::Signature),
    ES256(es256_signature::ES256Signature),
}

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
mod es256_signature;
mod key_pair;
mod private_key;
mod public_key;
mod signature;
