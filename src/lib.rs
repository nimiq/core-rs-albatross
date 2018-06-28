extern crate beserial;
#[macro_use]
extern crate beserial_derive;
extern crate blake2_rfc;
extern crate curve25519_dalek;
extern crate ed25519_dalek;
extern crate hex;
extern crate libargon2_sys;
extern crate rand;
extern crate sha2;

#[macro_use]
pub mod macros;

pub mod consensus;
pub mod network;
pub mod utils;

fn main() {
    println!("Hello, world!");
}
