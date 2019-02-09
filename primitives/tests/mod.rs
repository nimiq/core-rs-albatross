extern crate beserial;
extern crate nimiq_primitives as primitives;
#[cfg(feature = "nimiq-keys")]
extern crate nimiq_keys as keys;
#[cfg(feature = "nimiq-hash")]
extern crate nimiq_hash as hash;
#[cfg(feature = "lazy_static")]
#[macro_use]
extern crate lazy_static;
#[cfg(feature = "fixed-unsigned")]
extern crate fixed_unsigned;

#[cfg(feature = "account")]
#[cfg(feature = "transaction")]
mod account;
#[cfg(feature = "block")]
mod block;
#[cfg(feature = "transaction")]
mod transaction;
#[cfg(feature = "coin")]
mod coin;
