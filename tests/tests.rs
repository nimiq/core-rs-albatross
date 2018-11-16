extern crate curve25519_dalek;
extern crate beserial;
extern crate ed25519_dalek;
extern crate hex;
extern crate nimiq;
extern crate sha2;
extern crate bigdecimal;
extern crate num_traits;
extern crate num_bigint;
extern crate pretty_env_logger;

mod consensus;
mod network;
mod utils;

pub fn setup() {
    pretty_env_logger::try_init();
}
