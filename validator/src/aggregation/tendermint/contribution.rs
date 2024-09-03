use std::{collections::BTreeMap, ops, sync::Arc};

use nimiq_block::MultiSignature;
use nimiq_bls::{AggregateSignature, SecretKey};
use nimiq_collections::bitset::BitSet;
use nimiq_handel::{
    contribution::{AggregatableContribution, ContributionError},
    identity::WeightRegistry,
    update::LevelUpdate,
};
use nimiq_hash::Blake2sHash;
use nimiq_primitives::{policy::Policy, TendermintVote};
use nimiq_tendermint::{Aggregation, AggregationMessage};
use serde::{Deserialize, Serialize};

use crate::aggregation::registry::ValidatorRegistry;

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

impl AggregationMessage<Blake2sHash> for AggregateMessage {
    fn sender(&self) -> u16 {
        self.0.origin().try_into().expect("validator ID")
    }
}

pub(crate) fn finality_fn_from_validators(
    validators: Arc<ValidatorRegistry>,
) -> impl Fn(&TendermintContribution) -> bool {
    move |message: &TendermintContribution| {
        // Conditions for final contributions:
        // * Any proposal has 2f+1
        // (* Nil has f+1) would be implicit in the next condition
        // * The biggest proposal (NOT Nil) combined with the non cast votes is less than 2f+1

        let mut uncast_votes = Policy::SLOTS;
        let mut biggest_weight = 0;

        // Check every proposal present, including None
        for (hash, signature) in message.contributions.iter() {
            let weight = u16::try_from(validators.signers_weight(&signature.signers).unwrap_or(0))
                // Maybe this should panic as whenever the conversion from usize to u16 is impossible
                // due to overflow, this contribution is faulty as a whole and should never have ended up here.
                .unwrap_or(0);
            // Any option which has at least 2f+1 votes make this contribution final.
            if weight >= Policy::TWO_F_PLUS_ONE {
                return true;
            }
            // If the proposal does not have 2f+1 check if it has the new biggest_weight
            if hash.is_some() && weight > biggest_weight {
                biggest_weight = weight;
            }
            // The cast votes need to be removed from the uncast votes
            uncast_votes = uncast_votes.saturating_sub(weight);
        }

        let cast_votes = Policy::SLOTS.saturating_sub(uncast_votes);
        if cast_votes < Policy::TWO_F_PLUS_ONE {
            // This should not be here. The criteria that the best vote can be elevated above the threshold
            // using the uncast votes is enough. In our case that means that seeing f+1 different proposals
            // with 1 vote each is a final aggregate. That is currently not handled that way in tendermint
            // as it only looks at contributions above 2f contributors. That behavior is thus maintained
            // here, but can be removed in the future if tendermint is changed as well.
            return false;
        }

        // If the uncast votes are not able to put the biggest vote over the threshold
        // they cannot push any proposal over the threshold and thus the aggregate is final.
        if uncast_votes + biggest_weight < Policy::TWO_F_PLUS_ONE {
            return true;
        }

        false
    }
}
