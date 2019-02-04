#[macro_use]
extern crate beserial_derive;
#[macro_use]
extern crate nimiq_hash as hash;
#[macro_use]
extern crate nimiq_macros as macros;

use std::io;

use hex::FromHex;

use hash::{Blake2bHash, Blake2bHasher, Hasher, SerializeContent};

pub use self::key_pair::*;
pub use self::private_key::*;
pub use self::public_key::*;
pub use self::signature::*;
pub use self::errors::*;

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
                T: Borrow<$name>
        {
            fn sum<I>(iter: I) -> Self
                where
                    I: Iterator<Item = T>
            {
                $name(iter.fold($identity, |acc, item| acc + item.borrow().0))
            }
        }
    }
}

pub mod multisig;

mod errors;
mod key_pair;
mod private_key;
mod public_key;
mod signature;

create_typed_array!(Address, u8, 20);
hash_typed_array!(Address);
add_hex_io_fns_typed_arr!(Address, Address::SIZE);

impl From<Blake2bHash> for Address {
    fn from(hash: Blake2bHash) -> Self {
        let hash_arr: [u8; 32] = hash.into();
        return Address::from(&hash_arr[0..Address::len()]);
    }
}

impl<'a> From<&'a PublicKey> for Address {
    fn from(public_key: &'a PublicKey) -> Self {
        let hash = Blake2bHasher::default().digest(public_key.as_bytes());
        return Address::from(hash);
    }
}