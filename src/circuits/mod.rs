//! This module contains the zk-SNARK circuits that form the nano node program. Each circuit produces
//! a proof and they can be "chained" together by using one's output as another's input.

pub mod mnt4;
pub mod mnt6;
