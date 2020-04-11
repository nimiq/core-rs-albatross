//! This module contains the zk-SNARK circuits that form the nano node program. Each circuit produces
//! a proof and they can be "chained" together by using one's output as another's input.

//pub use dummy::DummyCircuit;
pub use macro_block::MacroBlockCircuit;
//pub use merger::MergerCircuit;
//pub use other_dummy::OtherDummyCircuit;
//pub use wrapper::WrapperCircuit;

// mod dummy;
mod macro_block;
// mod merger;
// mod other_dummy;
// mod wrapper;
