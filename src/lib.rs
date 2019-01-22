#![allow(dead_code)]
#![allow(unused_variables)]

extern crate beserial;
#[macro_use]
extern crate beserial_derive;
#[macro_use]
extern crate bitflags;
extern crate hex;
#[macro_use]
extern crate lazy_static;
#[macro_use]
extern crate log;
extern crate rand;
extern crate database;
extern crate parking_lot;
extern crate bit_vec;
extern crate unicode_normalization;
extern crate regex;
#[macro_use]
extern crate macros;
#[macro_use]
extern crate hash;

extern crate url;
extern crate byteorder;
extern crate futures;
extern crate tokio;
extern crate tokio_tls;
extern crate tokio_tungstenite;
extern crate native_tls;
extern crate tungstenite;
extern crate num_traits;
extern crate num_bigint;
extern crate bigdecimal;
extern crate weak_table;

pub mod consensus;
pub mod network;
pub mod utils;

// TODO: Remove?
fn main() {
    println!("Hello, world!");
}
