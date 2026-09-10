//! Consensus changes shipping in protocol version 3.
//!
//! Register every fork change here on its own line. The value is always
//! [`VERSION`] — never a literal — so a change that slips to a later fork is a
//! one-line move to that version's module.

/// The protocol version every change in this module activates in.
pub const VERSION: u16 = 3;

// --- Changes shipping in v3 -------------------------------------------------
// Add one `pub const MY_CHANGE: u16 = VERSION;` line per change below.

/// Prevent dusting-based DoS attacks on stake redelegation and retirement. `AddStake`
/// transactions must add at least the minimum stake and credit the active balance only if
/// it is already non-zero; otherwise, they credit the inactive balance without restarting
/// the cooldown period.
pub const STAKING_CHANGE_ADD_STAKE_POLICY: u16 = VERSION;
