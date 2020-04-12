//! This module contains the zk-SNARK circuits that use the MNT6-753 curve. This means that they
//! can manipulate elliptic curve points on the  MNT4-753 curve.

pub use dummy::DummyCircuit;
pub use other_dummy::OtherDummyCircuit;
pub use wrapper::WrapperCircuit;

pub mod dummy;
pub mod other_dummy;
pub mod wrapper;
