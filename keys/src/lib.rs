pub use nimiq_utils::key_rng::{SecureGenerate, SecureRng};

pub use self::{address::*, errors::*, key_pair::*, private_key::*, public_key::*, signature::*};

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
mod key_pair;
mod private_key;
mod public_key;
mod signature;
