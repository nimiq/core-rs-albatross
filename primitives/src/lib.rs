#[macro_use]
extern crate beserial_derive;
#[cfg(feature = "failure")]
#[macro_use]
extern crate failure;
#[macro_use]
extern crate log;
#[cfg(feature = "lazy_static")]
#[macro_use]
extern crate lazy_static;
#[cfg(feature = "nimiq-macros")]
extern crate nimiq_macros;
#[cfg(feature = "enum-display-derive")]
#[macro_use]
extern crate enum_display_derive;

#[cfg(feature = "coin")]
pub mod coin;
#[cfg(feature = "account")]
pub mod account;
#[cfg(feature = "policy")]
pub mod policy;
#[cfg(feature = "networks")]
pub mod networks;
#[cfg(feature = "validators")]
pub mod slot;
