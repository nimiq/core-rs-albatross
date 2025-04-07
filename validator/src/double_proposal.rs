use std::{
    collections::{btree_map, BTreeMap},
    mem,
};

use nimiq_block::{DoubleProposalProof, MacroHeader};
use nimiq_hash::Blake2sHash;
use nimiq_keys::{Address, Ed25519Signature as SchnorrSignature};
use nimiq_primitives::{networks::NetworkId, TendermintProposal};

struct Proposal {
    proposer: Address,
    hash: Blake2sHash,
    proposal: TendermintProposal<MacroHeader>,
    signature: SchnorrSignature,
}

enum Round {
    Seen(Proposal),
    Reported(Address),
}

impl Round {
    fn proposer(&self) -> &Address {
        match self {
            Round::Seen(Proposal { proposer, .. }) => proposer,
            Round::Reported(proposer) => proposer,
        }
    }
}

pub struct DoubleProposalDetector {
    network: NetworkId,
    block_number: u32,
    rounds: BTreeMap<u32, Round>,
}

impl DoubleProposalDetector {
    // TODO: add network_id
    // TODO: record one proposal per validator per height
    pub fn new(
        network: NetworkId,
        block_number: u32,
    ) -> DoubleProposalDetector {
        DoubleProposalDetector {
            network,
            block_number,
            rounds: BTreeMap::new(),
        }
    }
    pub fn observe_valid_proposal(
        &mut self,
        proposer: Address,
        proposal: TendermintProposal<MacroHeader>,
        signature: SchnorrSignature,
    ) -> Option<DoubleProposalProof> {
        assert_eq!(proposal.proposal.network, self.network);
        assert_eq!(proposal.proposal.block_number, self.block_number);
        match self.rounds.entry(proposal.round) {
            btree_map::Entry::Vacant(v) => {
                v.insert(Round::Seen(Proposal {
                    proposer,
                    hash: proposal.hash(),
                    proposal,
                    signature,
                }));
                None
            }
            btree_map::Entry::Occupied(o) => {
                let round = o.into_mut();
                assert_eq!(
                    *round.proposer(),
                    proposer,
                    "Only one address can propose in a round"
                );
                match round {
                    Round::Reported(_) => return None,
                    Round::Seen(Proposal { hash: old_hash, .. }) => {
                        if *old_hash == proposal.hash() {
                            return None;
                        }
                    }
                }
                match mem::replace(round, Round::Reported(proposer.clone())) {
                    Round::Reported(_) => unreachable!(),
                    Round::Seen(old_proposal) => Some(DoubleProposalProof::new(
                        proposer,
                        old_proposal.proposal,
                        old_proposal.signature,
                        proposal,
                        signature,
                    )),
                }
            }
        }
    }
}
