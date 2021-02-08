use std::sync::Arc;

use parking_lot::RwLock;

use crate::{contribution::AggregatableContribution, identity::{Identity, IdentityRegistry}};
use crate::identity::WeightRegistry;
use crate::partitioner::Partitioner;
use crate::store::ContributionStore;

pub trait Evaluator<C: AggregatableContribution>: Send + Sync {
    fn evaluate(&self, signature: &C, level: usize) -> usize;
    fn is_final(&self, signature: &C) -> bool;
    fn level_contains_id(&self, level: usize, id: usize) -> bool;
}

/// Every signature counts as a single vote
pub struct SingleVote<S: ContributionStore, P: Partitioner> {
    signature_store: Arc<S>,
    partitioner: Arc<P>,
    threshold: usize,
}

impl<S: ContributionStore, P: Partitioner> SingleVote<S, P> {
    pub fn new(signature_store: Arc<S>, partitioner: Arc<P>, threshold: usize) -> Self {
        Self {
            signature_store,
            partitioner,
            threshold,
        }
    }
}

impl<C: AggregatableContribution, S: ContributionStore<Contribution = C>, P: Partitioner> Evaluator<C> for SingleVote<S, P> {
    fn evaluate(&self, _contribution: &C, _level: usize) -> usize {
        // This is going to be used here, and we don't want any warnings
        let _ = (&self.signature_store, &self.partitioner);

        // TODO: The code from `WeightedVote` is what actually belongs here. And then the code in
        // `WeightedVote` must be adapted to consider the weight of a signature.
        unimplemented!();
    }

    fn is_final(&self, contribution: &C) -> bool {
        contribution.num_contributors() >= self.threshold
    }

    fn level_contains_id(&self, level: usize, id: usize) -> bool {
        self.partitioner.range(level).unwrap().contains(&id)
    }
}

/// A signature counts as it was signed N times, where N is the signers weight
///
/// NOTE: This can be used for ViewChanges
#[derive(Debug)]
pub struct WeightedVote<S: ContributionStore, I: WeightRegistry + IdentityRegistry, P: Partitioner> {
    store: Arc<RwLock<S>>,
    pub weights: Arc<I>,
    partitioner: Arc<P>,
    pub threshold: usize,
}

impl<S: ContributionStore, I: WeightRegistry + IdentityRegistry, P: Partitioner> WeightedVote<S, I, P> {
    pub fn new(store: Arc<RwLock<S>>, weights: Arc<I>, partitioner: Arc<P>, threshold: usize) -> Self {
        Self {
            store,
            weights,
            partitioner,
            threshold,
        }
    }
}

impl<C: AggregatableContribution, S: ContributionStore<Contribution = C>, I: WeightRegistry + IdentityRegistry, P: Partitioner> Evaluator<C> for WeightedVote<S, I, P> {
    /// takes an unverified contribution and scroes it in terms of usefulness with
    ///
    /// `0` being not useful at all, can be discarded.
    ///
    /// `>0` being more useful the bigger the number.
    fn evaluate(&self, contribution: &C, level: usize) -> usize {
        // TODO: Consider weight
        //let weight = self.weights.signature_weight(&signature)
        //    .unwrap_or_else(|| panic!("No weight for signature: {:?}", signature));

        let store = self.store.read();

        // check if we already know this individual signature
        // signers_identity returns None if it is not a single identity (also no identity at all)
        let identity = self.weights.signers_identity(&contribution.contributors());
        if let Identity::Single(identity) = identity {
            if store.individual_signature(level, identity).is_some() {
                // If we already know it for this level, score it as 0
                trace!(
                    "Individual contribution from peer {} for level {} already known",
                    level,
                    identity,
                );
                return 0;
            }
        }

        // number of identities at `level`, sort of maximum receivable contributions
        let to_receive = self.partitioner.level_size(level);
        let best_contribution = store.best(level);

        if let Some(best_contribution) = best_contribution {
            let best_contributors_num = match self.weights.signers_identity(&best_contribution.contributors()) {
                Identity::None => 0,
                Identity::Single(_) => 1,
                Identity::Multiple(ids) => ids.len(),
            };

            trace!("level = {}", level);
            trace!("contribution = {:#?}", contribution);
            trace!("best_contribution = {:#?} - Ids: {}", best_contribution, best_contributors_num);

            // check if the best signature for that level is already complete
            if to_receive == best_contributors_num {
                trace!("Best contribution already complete");
                return 0;
            }

            // check if the best signature is better than the new one
            if best_contribution.contributors().is_superset(&contribution.contributors()) {
                trace!("Best signature is better");
                return 0;
            }
        }

        // the signers of the signature
        // NOTE: This is a little bit more efficient than `signature.signers().collect()`, since
        // `signers()` returns a boxed iterator.
        // NOTE: We compute the full `BitSet` (also for individual signatures), since we need it in
        // a few places here
        let signers = if let Identity::Single(identity) = identity {
            let mut individuals = store.individual_verified(level).clone();
            individuals.insert(identity);
            individuals
        } else {
            contribution.contributors()
        };

        // compute bitset of signers combined with all (verified) individual signatures that we have
        let with_individuals = &signers | store.individual_verified(level);

        // ---------------------------------------------

        let (new_total, added_sigs, combined_sigs) = if let Some(best_signature) = best_contribution {
            let best_contributors_num = match self.weights.signers_identity(&best_signature.contributors()) {
                Identity::None => 0,
                Identity::Single(_) => 1,
                Identity::Multiple(ids) => ids.len(),
            };
            // TODO weights!
            if signers.intersection_size(&best_signature.contributors()) > 0 {
                // can't merge
                let new_total = with_individuals.len();
                (
                    new_total,
                    new_total.saturating_sub(best_contributors_num),
                    new_total - signers.len(),
                )
            } else {
                let final_sig = &with_individuals | &best_signature.contributors();
                let new_total = final_sig.len();
                let combined_sigs = (final_sig ^ (&best_signature.contributors() | &signers)).len();
                (new_total, new_total - best_contributors_num, combined_sigs)
            }
        } else {
            // best is the new signature with the individual signatures
            let new_total = with_individuals.len();
            (new_total, new_total, new_total - signers.len())
        };

        trace!("new_total={}, added_sigs={}, combined_sigs={}", new_total, added_sigs, combined_sigs);

        // compute score
        // TODO: Remove magic numbers! What do they mean? I don't think this is discussed in the paper.
        if added_sigs == 0 {
            // return signature_weight for an individual signature, otherwise 0
            if let Identity::Single(_) = self.weights.signers_identity(&contribution.contributors()) {
                self.weights.signature_weight(contribution).unwrap_or(0)
            } else {
                0
            }
        } else if new_total == to_receive {
            1_000_000 - level * 10 - combined_sigs
        } else {
            100_000 - level * 100 + added_sigs * 10 - combined_sigs
        }
    }

    fn is_final(&self, signature: &C) -> bool {
        let votes = self
            .weights
            .signature_weight(signature)
            .unwrap_or_else(|| panic!("Missing weights for signature: {:?}", signature));

        trace!("is_final(): votes={}, final={}", votes, votes >= self.threshold);
        votes >= self.threshold
    }

    fn level_contains_id(&self, level: usize, id: usize) -> bool {
        self.partitioner.range(level).unwrap().contains(&id)
    }
}
