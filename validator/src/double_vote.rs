use std::{
    collections::{btree_map, BTreeMap},
    mem,
};

use nimiq_block::{DoubleVoteProof, MultiSignature};
use nimiq_hash::Blake2sHash;
use nimiq_primitives::{
    networks::NetworkId, policy::Policy, slots_allocation::Validators, TendermintIdentifier,
    TendermintVote,
};

struct Vote {
    vote: Option<Blake2sHash>,
    signature: MultiSignature,
}

enum SlotBand {
    Seen(Vote),
    Reported,
}

pub struct DoubleVoteDetector {
    network: NetworkId,
    block_number: u32,
    validators: Validators,
    round_slot_bands: BTreeMap<(TendermintIdentifier, u16), SlotBand>,
}

impl DoubleVoteDetector {
    pub fn new(
        network: NetworkId,
        block_number: u32,
        validators: Validators,
    ) -> DoubleVoteDetector {
        DoubleVoteDetector {
            network,
            block_number,
            validators,
            round_slot_bands: BTreeMap::new(),
        }
    }
    pub fn observe_valid_vote(
        &mut self,
        vote: &TendermintVote,
        signature: &MultiSignature,
    ) -> Vec<DoubleVoteProof> {
        assert_eq!(vote.id.network, self.network);
        assert_eq!(vote.id.block_number, self.block_number);

        let &TendermintVote {
            id,
            proposal_hash: ref vote,
        } = vote;
        let mut result = Vec::new();
        for slot in signature.signers.iter() {
            assert!(slot < Policy::SLOTS as usize);
            let slot = u16::try_from(slot).unwrap();
            let slot_band = self.validators.get_band_from_slot(slot);
            if let Some(proof) =
                self.observe_valid_vote_from(slot_band, id, vote.clone(), signature)
            {
                result.push(proof)
            }
        }
        result
    }

    fn observe_valid_vote_from(
        &mut self,
        slot_band: u16,
        id: TendermintIdentifier,
        vote: Option<Blake2sHash>,
        signature: &MultiSignature,
    ) -> Option<DoubleVoteProof> {
        match self.round_slot_bands.entry((id, slot_band)) {
            btree_map::Entry::Vacant(v) => {
                v.insert(SlotBand::Seen(Vote {
                    vote: vote.clone(),
                    signature: signature.clone(),
                }));
                None
            }
            btree_map::Entry::Occupied(o) => {
                let entry = o.into_mut();
                match entry {
                    SlotBand::Reported => return None,
                    SlotBand::Seen(Vote { vote: old_vote, .. }) => {
                        if *old_vote == vote {
                            return None;
                        }
                    }
                }
                match mem::replace(entry, SlotBand::Reported) {
                    SlotBand::Reported => unreachable!(),
                    SlotBand::Seen(old) => Some(DoubleVoteProof::new(
                        id,
                        self.validators
                            .get_validator_by_slot_band(slot_band)
                            .address
                            .clone(),
                        old.vote,
                        old.signature.signature,
                        old.signature.signers,
                        vote.clone(),
                        signature.signature,
                        signature.signers.clone(),
                    )),
                }
            }
        }
    }
}
