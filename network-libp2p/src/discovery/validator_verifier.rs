use nimiq_keys::KeyPair;
use nimiq_network_interface::{
    network::Network as NetworkInterface, validator_claim::ValidatorClaim,
};
use nimiq_utils::tagged_signing::TaggedSigned;

use crate::Network;

/// A [`ValidatorClaim`] together with its signature, as reconstructed from a peer contact.
pub type SignedValidatorClaim =
    TaggedSigned<ValidatorClaim<<Network as NetworkInterface>::PeerId>, KeyPair>;

/// Why a validator claim could not be checked at this point in time.
///
/// These outcomes are transient: the same claim may verify later, once this node has the state
/// required to check it. Contacts carrying such a claim are kept and re-checked periodically.
///
/// Every current reason is *node-wide*: it depends only on this node's own state (whether it has
/// a verifier, runs a light blockchain, or has a complete staking contract), never on the claim
/// being checked. The verifiers return them before even looking at the claimed validator address.
/// So once one claim comes back with such a reason, every other claim checked right now would
/// too, and checking them only costs blockchain reads. See [`UnverifiableReason::is_node_wide`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnverifiableReason {
    /// This node has no verifier wired up (e.g. the web client).
    NoVerifier,
    /// This node runs a light blockchain and cannot read the staking contract.
    LightClient,
    /// The staking contract is not (yet) complete on this node.
    StateIncomplete,
}

impl UnverifiableReason {
    /// Whether this reason depends only on this node's own state, never on the claim that was
    /// checked, so that every other claim checked before this node's state changes would come back
    /// unverifiable as well.
    ///
    /// Callers use this to stop spending blockchain reads on further checks until the next
    /// house-keeping tick (see [`ValidatorClaimBudget::exhaust`]). The match is deliberately
    /// exhaustive: a future reason that depends on the claim itself (for example, a lookup that
    /// failed for this one validator only) must return `false` here, or a single such claim would
    /// stall verification of every other claim for the rest of the tick.
    pub fn is_node_wide(&self) -> bool {
        match self {
            UnverifiableReason::NoVerifier
            | UnverifiableReason::LightClient
            | UnverifiableReason::StateIncomplete => true,
        }
    }
}

/// Why a validator claim is considered bogus.
///
/// These outcomes are conclusive for the claim as presented, but they are *not* grounds for
/// dropping the connection: an honest validator can present a claim we reject, for example while
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
    /// Whether the claim could not be checked for a reason that applies to every claim alike.
    /// See [`UnverifiableReason::is_node_wide`].
    pub fn is_node_wide_unverifiable(&self) -> bool {
        matches!(self, ValidatorVerification::Unverifiable(reason) if reason.is_node_wide())
    }
}

/// Checks the validator claims carried by peer contacts against the staking contract.
///
/// This deliberately verifies *only* the binding of validator address to signing key and the
/// signature itself. Checks that are specific to DHT records (publisher identity, record key,
/// timestamp drift) stay in the DHT verifier, because a peer contact carries a different
/// timestamp unit and has no publisher.
pub trait ValidatorClaimVerifier: Send + Sync {
    fn verify_validator_claim(&self, signed_claim: &SignedValidatorClaim) -> ValidatorVerification;
}

/// A verifier for nodes that cannot check validator claims at all.
///
/// Every claim comes back as [`UnverifiableReason::NoVerifier`], so no contact is ever indexed as
/// belonging to a validator. A contact carrying a claim is still stored and can be dialed, but it
/// is never passed on to other peers (see
/// [`PeerContactInfo::is_gossipable`](super::peer_contacts::PeerContactInfo::is_gossipable)), so
/// a node using this verifier does not relay the contact of any validator that attaches a claim.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoopValidatorClaimVerifier;

impl ValidatorClaimVerifier for NoopValidatorClaimVerifier {
    fn verify_validator_claim(
        &self,
        _signed_claim: &SignedValidatorClaim,
    ) -> ValidatorVerification {
        ValidatorVerification::Unverifiable(UnverifiableReason::NoVerifier)
    }
}

#[cfg(test)]
mod tests {
    use nimiq_test_log::test;

    use super::*;

    #[test]
    fn every_current_unverifiable_reason_is_node_wide() {
        for reason in [
            UnverifiableReason::NoVerifier,
            UnverifiableReason::LightClient,
            UnverifiableReason::StateIncomplete,
        ] {
            assert!(reason.is_node_wide());
            assert!(ValidatorVerification::Unverifiable(reason).is_node_wide_unverifiable());
        }
        assert!(!ValidatorVerification::Verified.is_node_wide_unverifiable());
        assert!(
            !ValidatorVerification::Invalid(InvalidReason::InvalidSignature)
                .is_node_wide_unverifiable()
        );
    }
}
