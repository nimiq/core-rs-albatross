//! This module contains a series of cryptographic primitives that are off-circuit versions of the primitives
//! in the gadgets module. The main purpose of these primitives is to be used by other crates who might
//! need an off-circuit version of a primitive that is however guaranteed to be consistent with the on-circuit
//! primitive used by one of our zk-SNARKs. They are also used for testing.

pub mod mnt4;
