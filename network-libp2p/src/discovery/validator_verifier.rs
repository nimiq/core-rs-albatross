use std::sync::atomic::{AtomicUsize, Ordering};

use nimiq_keys::KeyPair;
use nimiq_network_interface::{
    network::Network as NetworkInterface, validator_record::ValidatorRecord,
};
use nimiq_utils::tagged_signing::TaggedSigned;

use crate::Network;

/// A [`ValidatorRecord`] together with its signature, as reconstructed from a peer contact.
pub type SignedValidatorRecord =
    TaggedSigned<ValidatorRecord<<Network as NetworkInterface>::PeerId>, KeyPair>;

/// Why a validator record could not be checked at this point in time.
///
/// These outcomes are transient: the same record may verify later, once this node has the state
/// required to check it. Contacts carrying such a claim are kept and re-checked periodically.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnverifiableReason {
    /// This node has no verifier wired up (e.g. the web client).
    NoVerifier,
    /// This node runs a light blockchain and cannot read the staking contract.
    LightClient,
    /// The staking contract is not (yet) complete on this node.
    StateIncomplete,
}

/// Why a validator record is considered bogus.
///
/// These outcomes are conclusive for the record as presented, but they are *not* grounds for
/// dropping the connection: an honest validator can present a record we reject, for example while
/// its registration transaction is still pending or right after a signing key rotation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InvalidReason {
    /// The staking contract has no validator with the claimed address.
    UnknownValidator,
    /// The signature does not verify against the validator's on-chain signing key.
    InvalidSignature,
}

/// The outcome of checking the validator claim carried by a peer contact.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValidatorVerification {
    /// The claim was checked against the staking contract and holds.
    Verified,
    /// The claim could not be checked yet. Re-check later.
    Unverifiable(UnverifiableReason),
    /// The claim was checked and does not hold.
    Invalid(InvalidReason),
}

impl ValidatorVerification {
    /// Whether the claim was positively verified.
    pub fn is_verified(&self) -> bool {
        matches!(self, ValidatorVerification::Verified)
    }
}

/// Checks the validator claims carried by peer contacts against the staking contract.
///
/// This deliberately verifies *only* the binding of validator address to signing key and the
/// signature itself. Checks that are specific to DHT records (publisher identity, record key,
/// timestamp drift) stay in the DHT verifier, because a peer contact carries a different
/// timestamp unit and has no publisher.
pub trait ValidatorRecordVerifier: Send + Sync {
    fn verify_validator_record(&self, signed_record: &SignedValidatorRecord)
        -> ValidatorVerification;
}

/// A verifier for nodes that cannot check validator records at all.
///
/// Every claim comes back as [`UnverifiableReason::NoVerifier`], so contacts keep working as plain
/// peer contacts and are never indexed as belonging to a validator.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoopValidatorRecordVerifier;

impl ValidatorRecordVerifier for NoopValidatorRecordVerifier {
    fn verify_validator_record(
        &self,
        _signed_record: &SignedValidatorRecord,
    ) -> ValidatorVerification {
        ValidatorVerification::Unverifiable(UnverifiableReason::NoVerifier)
    }
}

/// Bounds how many validator claims carried by peer contacts this node will verify against the
/// staking contract per house-keeping tick, across *all* discovery connections combined.
///
/// Verifying a claim takes the blockchain read lock, and an unauthenticated peer can hand us one
/// in its very first handshake message. Without a shared, aggregate cap, an attacker opening many
/// connections (each carrying up to [`Config::update_limit`](super::behaviour::Config) claims)
/// could force an unbounded burst of blockchain-state reads. A claim that doesn't fit in the
/// current budget is simply left unverified; the periodic re-check sweep
/// (`PeerContactBook::unverified_validator_contacts`) checks it later.
#[derive(Debug)]
pub struct ValidatorClaimBudget {
    remaining: AtomicUsize,
}

impl ValidatorClaimBudget {
    pub fn new(capacity: usize) -> Self {
        Self {
            remaining: AtomicUsize::new(capacity),
        }
    }

    /// Tries to consume one unit of budget. Returns whether one was available.
    pub fn try_consume(&self) -> bool {
        self.remaining
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |r| r.checked_sub(1))
            .is_ok()
    }

    /// Refills the budget for the next tick.
    pub fn reset(&self, capacity: usize) {
        self.remaining.store(capacity, Ordering::Relaxed);
    }
}
