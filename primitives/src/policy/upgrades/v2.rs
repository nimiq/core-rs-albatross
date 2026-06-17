//! Consensus changes shipping in protocol version 2.
//!
//! Register every fork change here on its own line. The value is always
//! [`VERSION`] — never a literal — so a change that slips to a later fork is a
//! one-line move to that version's module.

/// The protocol version every change in this module activates in.
pub const VERSION: u16 = 2;

// --- Changes shipping in v2 -------------------------------------------------
// Add one `pub const MY_CHANGE: u16 = VERSION;` line per change below.

// pub const HASH_CHANGE: u16 = VERSION;
// pub const STAKING_CHANGE_XX: u16 = VERSION;
