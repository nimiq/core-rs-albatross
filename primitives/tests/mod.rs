extern crate nimiq_primitives as primitives;
#[cfg(feature = "lazy_static")]
#[macro_use]
extern crate lazy_static;
#[cfg(feature = "fixed-unsigned")]
extern crate fixed_unsigned;

#[cfg(feature = "coin")]
mod coin;
