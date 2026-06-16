use std::fmt::Debug;

use nimiq_bls::{AggregatePublicKey, SigHash};
use nimiq_hash::Blake2bHash;
use nimiq_hash_derive::SerializeContent;
use nimiq_primitives::{
    networks::NetworkId,
    policy::{upgrades, Policy},
    slots_allocation::Validators,
    Message, SignedMessage, PREFIX_SKIP_BLOCK_INFO, PREFIX_SKIP_BLOCK_WITH_STATE_ROOT_INFO,
};
use nimiq_serde::{Deserialize, Serialize, SerializedMaxSize};
use nimiq_vrf::VrfEntropy;

use crate::{multisig::checked_signer_slots, MicroBlock, MultiSignature};

pub type SignedSkipBlockInfo = SignedMessage<SkipBlockInfo>;

/// A struct that represents the basic information of a skip block.
#[derive(
    Clone, Debug, Ord, PartialOrd, Eq, PartialEq, Deserialize, Serialize, SerializeContent,
)]
pub struct SkipBlockInfo {
    /// The network of this skip block.
    pub network_id: NetworkId,

    /// The number of the block for which the skip block is constructed.
    pub block_number: u32,

    /// The seed of the previous block. This is needed to distinguish skip blocks on different
    /// branches. We chose the seed so that the skip block applies to all branches of a malicious
    /// fork, but not to branching because of skip blocks.
    /// We use the seed entropy since that is what is actually unique, not the VRF seed itself.
    pub vrf_entropy: VrfEntropy,
}

impl SkipBlockInfo {
    pub fn from_micro_block(block: &MicroBlock) -> Option<Self> {
        if block.is_skip_block() {
            Some(SkipBlockInfo {
                network_id: block.header.network,
                block_number: block.header.block_number,
                vrf_entropy: block.header.seed.entropy(),
            })
        } else {
            None
        }
    }

    /// Computes the message hash that the validators sign when attesting to a skip block.
    ///
    /// From `upgrades::v2::SKIP_BLOCK_STATE_ROOT_BINDING` onwards the signed message is a
    /// [`SkipBlockWithStateRootInfo`], which additionally commits to the skip block's
    /// `state_root`. This binds the otherwise unauthenticated header commitment to the aggregate
    /// proof, so a valid proof can no longer be replayed onto a header carrying a forged
    /// `state_root`. For earlier protocol versions the legacy hash (over the reduced
    /// `SkipBlockInfo` only) is returned unchanged, keeping historical proofs verifiable.
    pub fn signing_hash(&self, state_root: &Blake2bHash, protocol_version: u16) -> SigHash {
        if protocol_version < upgrades::v2::SKIP_BLOCK_STATE_ROOT_BINDING {
            self.hash_with_prefix()
        } else {
            SkipBlockWithStateRootInfo {
                info: self.clone(),
                state_root: state_root.clone(),
            }
            .hash_with_prefix()
        }
    }
}

impl Message for SkipBlockInfo {
    const PREFIX: u8 = PREFIX_SKIP_BLOCK_INFO;
}

/// The skip block message that validators sign from
/// `upgrades::v2::SKIP_BLOCK_STATE_ROOT_BINDING` on: the reduced [`SkipBlockInfo`] together with
/// the deterministic `state_root` the skip block commits to. Signing this as its own prefixed
/// [`Message`] (instead of re-hashing the `SkipBlockInfo` hash with the `state_root` appended)
/// keeps the domain separation between validator-signed messages explicit.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize, SerializeContent)]
pub struct SkipBlockWithStateRootInfo {
    pub info: SkipBlockInfo,
    pub state_root: Blake2bHash,
}

impl Message for SkipBlockWithStateRootInfo {
    const PREFIX: u8 = PREFIX_SKIP_BLOCK_WITH_STATE_ROOT_INFO;
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, SerializedMaxSize)]
pub struct SkipBlockProof {
    // The aggregated signature of the validator's signatures for the skip block.
    pub sig: MultiSignature,
}

impl SkipBlockProof {
    /// Verifies the proof. This only checks that the proof is valid for this skip block, not that
    /// the skip block itself is valid.
    ///
    /// `state_root` and `protocol_version` are those of the block the proof is attached to. From
    /// `upgrades::v2::SKIP_BLOCK_STATE_ROOT_BINDING` onwards the `state_root` is part of the
    /// signed message (see [`SkipBlockInfo::signing_hash`]).
    pub fn verify(
        &self,
        skip_block: &SkipBlockInfo,
        state_root: &Blake2bHash,
        protocol_version: u16,
        validators: &Validators,
    ) -> bool {
        let signer_slots = match checked_signer_slots(&self.sig.signers) {
            Some(slots) => slots,
            None => {
                error!(
                    "SkipBlockProof verification failed: signer set contains out-of-range slots."
                );
                return false;
            }
        };

        // Check if there are enough votes.
        if signer_slots.len() < Policy::TWO_F_PLUS_ONE as usize {
            error!(
                "SkipBlockProof verification failed: Not enough slots signed the skip block message."
            );
            return false;
        }

        // Get the public key for each SLOT present in the signature and add them together to get
        // the aggregated public key.
        let mut agg_pk = AggregatePublicKey::new();
        for slot in signer_slots {
            let Some(pk) = validators
                .get_validator_by_slot_number(slot)
                .voting_key
                .uncompress()
            else {
                return false;
            };
            agg_pk.aggregate(pk);
        }

        // Verify the aggregated signature against our aggregated public key.
        agg_pk.verify_hash(
            skip_block.signing_hash(state_root, protocol_version),
            &self.sig.signature,
        )
    }
}
