#![allow(dead_code)]
#![allow(unused_variables)]

extern crate beserial;
#[macro_use]
extern crate beserial_derive;
#[macro_use]
extern crate bitflags;
extern crate blake2_rfc;
extern crate curve25519_dalek;
extern crate ed25519_dalek;
extern crate hex;
#[macro_use]
extern crate lazy_static;
extern crate libargon2_sys;
#[macro_use]
extern crate log;
extern crate rand;
extern crate sha2;
extern crate lmdb_zero;
extern crate fs2;
extern crate parking_lot;

extern crate url;
extern crate byteorder;
extern crate futures;
extern crate tokio;
extern crate tokio_tungstenite;
extern crate tungstenite;
extern crate bit_vec;

#[macro_use]
pub mod macros;

pub mod consensus;
pub mod network;
pub mod utils;

fn main() {
    println!("Hello, world!");
}
