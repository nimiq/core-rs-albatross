//! This module contains several constants that are used throughout the library. They can be changed
//! easily here, without needing to change them in several places in the code.
use nimiq_primitives::policy;

/// This is the depth of the PKTree circuit.
pub const PK_TREE_DEPTH: usize = 5;

/// This is the number of leaves in the PKTree circuit.
pub const PK_TREE_BREADTH: usize = 2_usize.pow(PK_TREE_DEPTH as u32);

/// This is the length of one epoch in Albatross. Basically, the difference in the block numbers of
/// two consecutive macro blocks.
pub const EPOCH_LENGTH: u32 = policy::EPOCH_LENGTH;

/// This is the number of validator slots in Albatross.
/// VALIDATOR_SLOTS = MIN_SIGNERS + MAX_NON_SIGNERS
pub const VALIDATOR_SLOTS: usize = policy::SLOTS as usize;

/// This is the minimum number of validator slots that must sign a macro block in order to be valid.
pub const MIN_SIGNERS: usize = policy::TWO_THIRD_SLOTS as usize;
