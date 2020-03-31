//! This module contains all of the "gadgets" for the zk-SNARK circuits. These are smaller, modular pieces
//! of on-circuit code intended to facilitate the creation of larger circuits.

pub use alloc_constant::AllocConstantGadget;
pub use check_sig::CheckSigGadget;
pub use crh::{CRHGadget, CRHGadgetParameters};
pub use macro_block::{MacroBlockGadget, Round};
pub use state_hash::StateHashGadget;
pub use utils::{bytes_to_bits, hash_to_bits, pad_point_bits, reverse_inner_byte_order};
pub use y_to_bit::YToBitGadget;

mod alloc_constant;
mod check_sig;
mod crh;
mod macro_block;
mod state_hash;
mod utils;
mod y_to_bit;
