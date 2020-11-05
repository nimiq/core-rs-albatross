#![feature(async_closure)]
#![feature(drain_filter)]

#[macro_use]
extern crate beserial_derive;
#[macro_use]
extern crate log;

pub(crate) mod network;
pub(crate) mod outside_deps;
pub(crate) mod protocol;
pub(crate) mod state;
pub(crate) mod utils;
