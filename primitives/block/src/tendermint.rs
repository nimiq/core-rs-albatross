use log::error;
use nimiq_bls::AggregatePublicKey;
use nimiq_primitives::{
    policy::Policy, slots_allocation::Validators, TendermintIdentifier, TendermintStep,
    TendermintVote,
};
use nimiq_serde::{Deserialize, Serialize, SerializedMaxSize};

use crate::{multisig::checked_signer_slots, MacroBlock, MultiSignature};

/// The proof for a block produced by Tendermint.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, SerializedMaxSize)]
pub struct TendermintProof {
    // The round when the block was completed. This is necessary to verify the signature.
    pub round: u32,
    // The aggregated signature of the precommits of the validators for this block.
    pub sig: MultiSignature,
}

impl TendermintProof {
    /// This simply returns the number of slots that voted (precommitted) to this block.
    pub fn votes(&self) -> u16 {
        self.sig.signers.len() as u16
    }

    /// Verifies the proof. This only checks that the proof is valid for this block, not that the
    /// block itself is valid.
    pub fn verify(block: &MacroBlock, current_validators: &Validators) -> bool {
        // If there's no justification then the proof is false evidently.
        let justification = match &block.justification {
            None => {
                error!("Invalid justification - macro block has no justification!");
                return false;
            }
            Some(x) => x,
        };

        let signer_slots = match checked_signer_slots(&justification.sig.signers) {
            Some(slots) => slots,
            None => {
                error!("Invalid justification - signer set contains out-of-range slots!");
                return false;
            }
        };

        // Check if there are enough votes.
        if signer_slots.len() < Policy::TWO_F_PLUS_ONE as usize {
            error!("Invalid justification - not enough votes!");
            return false;
        }

        // Calculate the `block_hash` as blake2s.
        let block_hash = block.hash_blake2s();

        // Calculate the message that was actually signed by the validators.
        let message = TendermintVote {
            proposal_hash: Some(block_hash),
            id: TendermintIdentifier {
                network: block.network(),
                block_number: block.block_number(),
                round_number: justification.round,
                step: TendermintStep::PreCommit,
            },
        };

        // Get the public key for each SLOT and add them together to get the aggregated public key
        // (if they are part of the Multisignature Bitset).
        let mut agg_pk = AggregatePublicKey::new();

        for slot in signer_slots {
            if let Some(pk) = current_validators
                .get_validator_by_slot_number(slot)
                .voting_key
                .uncompress()
            {
                agg_pk.aggregate(pk);
            } else {
                return false;
            }
        }

        // Verify the aggregated signature against our aggregated public key.
        agg_pk.verify(&message, &justification.sig.signature)
    }
}
