//! Version-dependent test helper.
//!
//! [`assert_all_versions`] sweeps every protocol version from `1` up to
//! [`Policy::max_supported_version()`] (inclusive) and asserts a property that
//! may change at one or more *breakpoints*. This mirrors the version-gating
//! pattern used throughout the codebase, where behaviour flips on
//! `block.version() >= upgrades::vN::SOME_CHANGE` (see
//! [`nimiq_primitives::policy::upgrades`]).
//!
//! A *breakpoint* `(b, pred)` switches the active predicate to `pred` from
//! version `b` onward (inclusive). Before the first breakpoint, the
//! `before_breakpoint` predicate is active. Every version is evaluated with its
//! active predicate, which must return `true`.
//!
//! The helper is intentionally self-contained: apart from reading the
//! [`Policy`] constant for the upper bound it has no dependency on the rest of
//! the workspace, so the same pattern can be lifted into other projects.
//!
//! # Examples
//!
//! A property that flips once, at version 2 (predicates are pure functions of
//! `v`, so closures here capture nothing):
//!
//! ```ignore
//! use nimiq_test_utils::versions::{assert_all_versions, bp};
//!
//! assert_all_versions(
//!     |v| reward_curve(v) == Curve::Legacy,           // v == 1
//!     vec![bp(2, |v| reward_curve(v) == Curve::New)], // v == 2..=max
//! );
//! ```
//!
//! Several breakpoints produce several segments:
//!
//! ```ignore
//! assert_all_versions(
//!     |v| hash_algo(v) == Hash::Blake2b,           // v == 1
//!     vec![
//!         bp(2, |v| hash_algo(v) == Hash::Blake2s), // v == 2
//!         bp(3, |v| hash_algo(v) == Hash::Blake3),  // v == 3..=max
//!     ],
//! );
//! ```
//!
//! Bigger predicates may borrow a shared fixture (enabled by the lifetime on
//! [`VersionPredicate`]) and fold several checks into the returned `bool`:
//!
//! ```ignore
//! let blockchain = temporary_blockchain(NetworkId::UnitAlbatross);
//! let producer = BlockProducer::new(/* … */);
//!
//! assert_all_versions(
//!     |v| {
//!         let block = producer.next_macro_block(&blockchain, v, None);
//!         block.verify(&blockchain).is_ok() && block.diff_root() == Blake2bHash::default()
//!     },
//!     vec![bp(upgrades::v2::VERSION, |v| {
//!         let diff = compute_diff(&blockchain);
//!         let block = producer.next_macro_block(&blockchain, v, Some(diff.root()));
//!         block.verify(&blockchain).is_ok() && block.diff_root() == diff.root()
//!     })],
//! );
//! ```
//!
//! When a property is uniform across versions, omit the breakpoints entirely:
//!
//! ```ignore
//! assert_all_versions(|v| supports_upgrade_for(v), vec![]);
//! ```

use nimiq_primitives::policy::Policy;

/// A predicate evaluated for a single protocol version. It receives the version
/// `v` and must return `true`; returning `false` fails the assertion.
///
/// The `'a` lifetime allows boxed predicates to borrow test-local fixtures
/// rather than forcing every captured value to be `'static`.
pub type VersionPredicate<'a> = Box<dyn Fn(u16) -> bool + 'a>;

/// Convenience constructor for a breakpoint entry, avoiding the `Box::new`
/// boilerplate at call sites: `bp(2, |v| ...)` instead of
/// `(2, Box::new(|v| ...))`.
pub fn bp<'a>(version: u16, pred: impl Fn(u16) -> bool + 'a) -> (u16, VersionPredicate<'a>) {
    (version, Box::new(pred))
}

/// Asserts a version-gated property across every protocol version from `1` up
/// to [`Policy::max_supported_version()`] (inclusive).
///
/// `before_breakpoint` is the active predicate until the first breakpoint. Each
/// `(b, pred)` in `breakpoints` switches the active predicate to `pred` from
/// version `b` onward. `breakpoints` is sorted ascending internally, so the
/// caller need not pre-sort it.
///
/// # Panics
///
/// - if two breakpoints share the same version, or
/// - if any breakpoint `b` does not satisfy `1 < b <= max_version`, or
/// - if any version's active predicate returns `false` (the panic message lists
///   every failing version together with its active segment).
pub fn assert_all_versions<'a>(
    before_breakpoint: impl Fn(u16) -> bool,
    breakpoints: Vec<(u16, VersionPredicate<'a>)>,
) {
    assert_all_versions_with_max(
        Policy::max_supported_version(),
        before_breakpoint,
        breakpoints,
    );
}

/// Core of [`assert_all_versions`] with an explicit `max_version`, so the logic
/// can be exercised deterministically without touching the global [`Policy`].
fn assert_all_versions_with_max<'a>(
    max_version: u16,
    before_breakpoint: impl Fn(u16) -> bool,
    mut breakpoints: Vec<(u16, VersionPredicate<'a>)>,
) {
    assert!(max_version >= 1, "max_version must be at least 1");

    // Sort ascending by version so segment selection and the duplicate check
    // below can rely on order.
    breakpoints.sort_by_key(|(version, _)| *version);

    // Validate: breakpoints must be unique and in range.
    for pair in breakpoints.windows(2) {
        assert_ne!(
            pair[0].0, pair[1].0,
            "duplicate breakpoint at version {}",
            pair[0].0
        );
    }
    for (version, _) in &breakpoints {
        assert!(
            *version > 1 && *version <= max_version,
            "breakpoint {version} out of range; must satisfy 1 < breakpoint <= {max_version}"
        );
    }

    // Evaluate every version with its active predicate, collecting all failures
    // so the panic reports them together rather than stopping at the first.
    let mut failures = Vec::new();
    for v in 1..=max_version {
        // The active predicate is the one of the highest breakpoint <= v, or
        // `before_breakpoint` if no breakpoint has been reached yet.
        match breakpoints.iter().rev().find(|(b, _)| *b <= v) {
            Some((b, pred)) => {
                if !pred(v) {
                    failures.push(format!("version {v} (breakpoint at version {b})"));
                }
            }
            None => {
                if !before_breakpoint(v) {
                    failures.push(format!("version {v} (before first breakpoint)"));
                }
            }
        }
    }

    assert!(
        failures.is_empty(),
        "version assertion failed for: {}",
        failures.join(", ")
    );
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;

    #[test]
    fn selects_correct_segment_and_sorts_input() {
        // max = 5, segments: before -> {1}, bp(2) -> {2, 3}, bp(4) -> {4, 5}.
        // Each predicate returns true only for versions in its own segment, so
        // a wrong segment selection (or a failure to sort) would fail.
        assert_all_versions_with_max(
            5,
            |v| v == 1,
            // Passed out of order on purpose to exercise the internal sort.
            vec![
                bp(4, |v| (4..=5).contains(&v)),
                bp(2, |v| (2..=3).contains(&v)),
            ],
        );
    }

    #[test]
    fn breakpoint_equal_to_max_version_is_inclusive() {
        // bp(3) is active only for the final version.
        assert_all_versions_with_max(3, |v| v < 3, vec![bp(3, |v| v == 3)]);
    }

    #[test]
    fn empty_breakpoints_cover_all_versions() {
        assert_all_versions_with_max(4, |v| (1..=4).contains(&v), vec![]);
    }

    #[test]
    fn max_version_of_one() {
        assert_all_versions_with_max(1, |v| v == 1, vec![]);
    }

    #[test]
    fn every_version_is_visited_exactly_once() {
        let visited = RefCell::new(Vec::new());
        assert_all_versions_with_max(
            4,
            |v| {
                visited.borrow_mut().push(v);
                true
            },
            vec![bp(3, |v| {
                visited.borrow_mut().push(v);
                true
            })],
        );
        assert_eq!(*visited.borrow(), vec![1, 2, 3, 4]);
    }

    #[test]
    #[should_panic(expected = "duplicate breakpoint at version 2")]
    fn duplicate_breakpoint_panics() {
        assert_all_versions_with_max(3, |_| true, vec![bp(2, |_| true), bp(2, |_| true)]);
    }

    #[test]
    #[should_panic(expected = "out of range")]
    fn breakpoint_above_max_panics() {
        assert_all_versions_with_max(3, |_| true, vec![bp(5, |_| true)]);
    }

    #[test]
    #[should_panic(expected = "out of range")]
    fn breakpoint_at_one_panics() {
        assert_all_versions_with_max(3, |_| true, vec![bp(1, |_| true)]);
    }

    #[test]
    #[should_panic(expected = "out of range")]
    fn breakpoint_at_zero_panics() {
        assert_all_versions_with_max(3, |_| true, vec![bp(0, |_| true)]);
    }

    #[test]
    #[should_panic(expected = "version 2 (before first breakpoint)")]
    fn failing_before_breakpoint_names_version_and_segment() {
        assert_all_versions_with_max(3, |v| v != 2, vec![]);
    }

    #[test]
    #[should_panic(expected = "version 2 (breakpoint at version 2)")]
    fn failing_breakpoint_segment_names_breakpoint() {
        assert_all_versions_with_max(3, |_| true, vec![bp(2, |_| false)]);
    }

    #[test]
    fn predicates_can_borrow_a_shared_fixture() {
        // Demonstrates the `'a` lifetime: both predicates borrow `fixture`.
        let fixture = [10u16, 20, 30];
        assert_all_versions_with_max(
            3,
            |v| fixture[(v - 1) as usize] == v * 10,
            vec![bp(2, |v| fixture[(v - 1) as usize] == v * 10)],
        );
    }
}
