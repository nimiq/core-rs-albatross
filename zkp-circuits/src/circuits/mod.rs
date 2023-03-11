//! This module contains the zk-SNARK circuits that are used in the light macro sync. Each circuit produces
//! a proof and they can be "chained" together by using one's output as another's input.

pub mod mnt4;
pub mod mnt6;
