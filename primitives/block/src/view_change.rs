use crate::{Message, MultiSignature, SignedMessage, PREFIX_VIEW_CHANGE};
use beserial::{Deserialize, Serialize};
use nimiq_bls::AggregatePublicKey;
use nimiq_hash::{Hash, SerializeContent};
use nimiq_hash_derive::SerializeContent;
use nimiq_primitives::policy::TWO_THIRD_SLOTS;
use nimiq_primitives::slots::Validators;
use nimiq_vrf::VrfSeed;
use std::fmt;

/// The struct representing a view change. View changes happen when a given micro block is not
/// produced in time by its intended producer. It allows the next slot owner to take over and
/// produce the block. A proof is necessary but it exists as the ViewChangeProof struct.
#[derive(
    Clone, Debug, Ord, PartialOrd, Eq, PartialEq, Serialize, Deserialize, SerializeContent,
)]
pub struct ViewChange {
    /// The number of the block for which the view change is constructed (i.e. the block number
    /// the validator is at + 1, since it's for the next block).
    pub block_number: u32,

    /// The view number after the view change (i.e. the current view number + 1, except if the view
    /// change is for the first micro block of an batch, in that case it is simply 1).
    pub new_view_number: u32,

    /// The seed of the previous block. This is needed to distinguish view changes on different
    /// branches. We chose the seed so that the view change applies to all branches of a malicious
    /// fork, but not to branching because of view changes.
    pub prev_seed: VrfSeed,
}

impl Message for ViewChange {
    const PREFIX: u8 = PREFIX_VIEW_CHANGE;
}

impl Hash for ViewChange {}

pub type SignedViewChange = SignedMessage<ViewChange>;

/// A struct that represents a series of consecutive view changes at the same block height.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ViewChanges {
    /// The block number at which the view changes happened.
    pub block_number: u32,
    /// The first view number that was changed
    pub first_view_number: u32,
    /// The last view number - i.e. the first one that wasn't changed
    pub last_view_number: u32,
}

impl ViewChanges {
    pub fn new(
        block_number: u32,
        first_view_number: u32,
        last_view_number: u32,
    ) -> Option<ViewChanges> {
        if first_view_number < last_view_number {
            Some(ViewChanges {
                block_number,
                first_view_number,
                last_view_number,
            })
        } else {
            None
        }
    }
}

impl fmt::Display for ViewChange {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(
            f,
            "#{}.{} ({})",
            self.block_number, self.new_view_number, self.prev_seed
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "serde-derive", derive(serde::Serialize, serde::Deserialize))]
pub struct ViewChangeProof {
    // The aggregated signature of the validator's signatures for the view change.
    pub sig: MultiSignature,
}

impl ViewChangeProof {
    /// Verifies the proof. This only checks that the proof is valid for this view change, not that
    /// the view change itself is valid.
    pub fn verify(&self, view_change: &ViewChange, validators: &Validators) -> bool {
        // Check if there are enough votes.
        if self.sig.signers.len() < TWO_THIRD_SLOTS as usize {
            error!("ViewChangeProof verification failed: Not enough slots signed the view change.");
            return false;
        }

        // Get the public key for each SLOT present in the signature and add them together to get
        // the aggregated public key.
        let agg_pk =
            self.sig
                .signers
                .iter()
                .fold(AggregatePublicKey::new(), |mut aggregate, slot| {
                    let pk = validators
                        .get_validator(slot as u16)
                        .public_key
                        .uncompress()
                        .expect("Failed to uncompress CompressedPublicKey");
                    aggregate.aggregate(&pk);
                    aggregate
                });

        // Verify the aggregated signature against our aggregated public key.
        agg_pk.verify_hash(view_change.hash_with_prefix(), &self.sig.signature)
    }
}
