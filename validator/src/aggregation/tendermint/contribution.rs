use std::{collections::BTreeMap, ops};

use nimiq_block::{MultiSignature, TendermintVote};
use nimiq_bls::{AggregateSignature, SecretKey};
use nimiq_collections::bitset::BitSet;
use nimiq_handel::{
    contribution::{AggregatableContribution, ContributionError},
    update::LevelUpdate,
};
use nimiq_hash::Blake2sHash;
use nimiq_tendermint::Aggregation;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone)]
pub struct TendermintContribution {
    pub contributions: BTreeMap<Option<Blake2sHash>, MultiSignature>,
}

impl std::fmt::Debug for TendermintContribution {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        let mut debug = f.debug_map();
        debug.entries(self.contributions.iter().map(|(v, m)| (v, &m.signers)));
        debug.finish()
    }
}

impl TendermintContribution {
    pub(crate) fn from_vote(
        vote: TendermintVote,
        secret_key: &SecretKey,
        validator_slots: ops::Range<u16>,
    ) -> Self {
        assert!(!validator_slots.is_empty());
        // sign the hash
        let signature = AggregateSignature::from_signatures(&[secret_key
            .sign(&vote)
            .multiply(validator_slots.len() as u16)]);

        // get the slots of the validator and insert them into the bitset
        let mut signers = BitSet::new();
        for slot in validator_slots {
            signers.insert(slot as usize);
        }

        // create a MultiSignature with the single contributor `validator_address`
        let multi_signature = MultiSignature::new(signature, signers);

        let mut contributions = BTreeMap::new();
        contributions.insert(vote.proposal_hash, multi_signature);
        Self { contributions }
    }
}

impl AggregatableContribution for TendermintContribution {
    /// Combines two TendermintContributions. Every different proposal is represented with its own multisignature.
    /// When combining non existing keys, they must be inserted while the multisignatures of existing keys are combined.
    fn combine(&mut self, other_contribution: &Self) -> Result<(), ContributionError> {
        // TODO: If we don't need the overlapping IDs for the error, we can use `intersection_size`
        let overlap = &self.contributors() & &other_contribution.contributors();

        if overlap.is_empty() {
            // iterate over all different values the other_contributions HashMap contains.
            other_contribution.contributions.iter().for_each(|(hash, other_sig)| {
                // for every entry there
                self.contributions
                    // get the entry here
                    .entry(hash.clone())
                    // and update it
                    .and_modify(|sig|
                        // by combining both Multisigs
                        sig
                            .combine(other_sig)
                            .expect("Non Overlapping TendermintContribution encountered overlapping MultiSignatures"))
                    // if that entry does not exist, create it with the MultiSignature from other_contribution.
                    .or_insert_with(|| other_sig.clone());
            });
            Ok(())
        } else {
            Err(ContributionError::Overlapping(overlap))
        }
    }

    fn contributors(&self) -> BitSet {
        self.contributions
            .iter()
            .fold(BitSet::new(), |mut aggregated_set, multi_sig| {
                aggregated_set |= multi_sig.1.contributors();
                aggregated_set
            })
    }
}

impl Aggregation<Blake2sHash> for TendermintContribution {
    fn all_contributors(&self) -> BitSet {
        self.contributors()
    }
    fn contributors_for(&self, vote: Option<&Blake2sHash>) -> BitSet {
        if let Some(multisig) = self.contributions.get(&vote.cloned()) {
            multisig.contributors()
        } else {
            BitSet::default()
        }
    }
    fn proposals(&self) -> Vec<(Blake2sHash, usize)> {
        self.contributions
            .iter()
            .filter_map(|(hash_opt, sig)| {
                hash_opt
                    .as_ref()
                    .map(|hash| (hash.clone(), sig.contributors().len()))
            })
            .collect()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AggregateMessage(pub(crate) LevelUpdate<TendermintContribution>);

impl Aggregation<Blake2sHash> for AggregateMessage {
    fn all_contributors(&self) -> BitSet {
        self.0.aggregate.all_contributors()
    }
    fn contributors_for(&self, vote: Option<&Blake2sHash>) -> BitSet {
        self.0.aggregate.contributors_for(vote)
    }
    fn proposals(&self) -> Vec<(Blake2sHash, usize)> {
        self.0.aggregate.proposals()
    }
}
