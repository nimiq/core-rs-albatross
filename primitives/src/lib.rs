#[macro_use]
extern crate beserial_derive;

#[cfg(feature = "account")]
pub mod account;
#[cfg(feature = "coin")]
pub mod coin;
#[cfg(feature = "networks")]
pub mod networks;
#[cfg(feature = "policy")]
pub mod policy;
#[cfg(feature = "slots")]
pub mod slots;
