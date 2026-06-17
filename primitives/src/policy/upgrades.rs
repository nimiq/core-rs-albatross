//! Per-fork activation registries.
//!
//! Each `vN` submodule lists every consensus-affecting change that ships in
//! protocol version `N`. To gate code on a change, compare the *block's*
//! version (`block.version()`) against the change's constant:
//!
//! ```ignore
//! use nimiq_primitives::policy::upgrades;
//!
//! if block.version() >= upgrades::v2::HASH_CHANGE {
//!     // new behaviour
//! }
//! ```
//!
//! Conventions for contributors:
//! - Register a change by adding one `pub const MY_CHANGE: u16 = VERSION;` line
//!   to the current fork's `vN` module. One line per change keeps merge
//!   conflicts minimal.
//! - Always gate with `>=` (active from version `N` onward), never `==` — an
//!   `==` check would silently turn the change off again once the next fork
//!   ships.
//! - If a change slips to a later fork, move its line to that version's module.
//!   Because the value is `VERSION` (not a literal), it auto-corrects.

pub mod v2;
pub mod v3;
// pub mod v4;  // uncomment when the v4 fork opens
