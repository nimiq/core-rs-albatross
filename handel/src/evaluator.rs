use std::sync::Arc;

use parking_lot::RwLock;

use crate::multisig::Signature;
use crate::identity::WeightRegistry;
use crate::store::SignatureStore;
use crate::partitioner::Partitioner;



pub trait Evaluator {
    fn evaluate(&self, signature: &Signature, level: usize) -> usize;
    fn is_final(&self, signature: &Signature) -> bool;
}


/// Every signature counts as a single vote
pub struct SingleVote<S: SignatureStore, P: Partitioner> {
    signature_store: Arc<S>,
    partitioner: Arc<P>,
    threshold: usize,
}

impl<S: SignatureStore, P: Partitioner> SingleVote<S, P> {
    pub fn new(signature_store: Arc<S>, partitioner: Arc<P>, threshold: usize) -> Self {
        Self {
            signature_store,
            partitioner,
            threshold,
        }
    }
}

impl<S: SignatureStore, P: Partitioner> Evaluator for SingleVote<S, P> {
    fn evaluate(&self, _signature: &Signature, _level: usize) -> usize {
        // TODO: The code from `WeightedVote` is what actually belongs here. And then the code in
        // `WeightedVote` must be adapted to consider the weight of a signature.
        unimplemented!()
    }

    fn is_final(&self, signature: &Signature) -> bool {
        match signature {
            Signature::Individual(_) => self.threshold <= 1,
            Signature::Multi(multisig) => multisig.len() >= self.threshold
        }
    }
}



/// A signature counts as it was signed N times, where N is the signers weight
///
/// NOTE: This can be used for ViewChanges
pub struct WeightedVote<S: SignatureStore, I: WeightRegistry, P: Partitioner> {
    store: Arc<RwLock<S>>,
    pub weights: Arc<I>,
    partitioner: Arc<P>,
    pub threshold: usize
}

impl<S: SignatureStore, I: WeightRegistry, P: Partitioner> WeightedVote<S, I, P> {
    pub fn new(store: Arc<RwLock<S>>, weights: Arc<I>, partitioner: Arc<P>, threshold: usize) -> Self {
        Self {
            store,
            weights,
            partitioner,
            threshold,
        }
    }
}

impl<S: SignatureStore, I: WeightRegistry, P: Partitioner> Evaluator for WeightedVote<S, I, P> {
    fn evaluate(&self, signature: &Signature, level: usize) -> usize {
        // TODO: Consider weight
        //let weight = self.weights.signature_weight(&signature)
        //    .unwrap_or_else(|| panic!("No weight for signature: {:?}", signature));

        let store = self.store.read();

        // check if we already know this individual signature
        match signature {
            Signature::Individual(individual) => {
                if store.individual_signature(level, individual.signer).is_some() {
                    // If we already know it for this level, score it as 0
                    trace!("Individual signature from peer {} for level {} already known", level, individual.signer);
                    return 0
                }
            },
            _ => {}
        }

        let to_receive = self.partitioner.level_size(level);
        let best_signature = store.best(level);

        if let Some(best_signature) = best_signature {
            trace!("level = {}", level);
            trace!("signature = {:#?}", signature);
            trace!("best_signature = {:#?}", best_signature);

            // check if the best signature for that level is already complete
            if to_receive == best_signature.len() {
                trace!("Best signature already complete");
                return 0;
            }

            // check if the best signature is better than the new one
            let best_is_better = match signature {
                Signature::Individual(individual) => best_signature.signers.contains(individual.signer),
                Signature::Multi(multisig) => best_signature.signers.is_superset(&multisig.signers),
            };
            if best_is_better {
                trace!("Best signature is better");
                return 0;
            }
        }

        // the signers of the signature
        // NOTE: This is a little bit more efficient than `signature.signers().collect()`, since
        // `signers()` returns a boxed iterator.
        // NOTE: We compute the full `BitSet` (also for individual signatures), since we need it in
        // a few places here
        let signers = match signature {
            Signature::Individual(individual) => {
                let mut individuals = store.individual_verified(level).clone();
                individuals.insert(individual.signer);
                individuals
            },
            Signature::Multi(multisig) => multisig.signers.clone()
        };

        // compute bitset of signers combined with all (verified) individual signatures that we have
        let with_individuals = &signers | &store.individual_verified(level);

        // ---------------------------------------------

        let (new_total, added_sigs, combined_sigs) = if let Some(best_signature) = best_signature {
            if signers.intersection_size(&best_signature.signers) > 0 {
                // can't merge
                let new_total = with_individuals.len();
                (new_total, new_total.saturating_sub(best_signature.len()), new_total - signers.len())
            }
            else {
                let final_sig = &with_individuals | &best_signature.signers;
                let new_total = final_sig.len();
                let combined_sigs = (final_sig ^ (&best_signature.signers | &signers)).len();
                (new_total, new_total - best_signature.len(), combined_sigs)
            }
        }
        else {
            // best is the new signature with the individual signatures
            let new_total = with_individuals.len();
            (new_total, new_total, new_total - signers.len())
        };

        trace!("new_total={}, added_sigs={}, combined_sigs={}", new_total, added_sigs, combined_sigs);

        // compute score
        // TODO: Remove magic numbers! What do they mean???? I don't think this is discussed in the paper.
        let score = if added_sigs == 0 {
            // return 1 for an individual signature, otherwise 0
            match signature {
                Signature::Individual(_) => 1,
                Signature::Multi(_) => 0,
            }
        }
        else if new_total == to_receive {
            1000000 - level * 10 - combined_sigs
        }
        else {
            100000 - level * 100 + added_sigs * 10 - combined_sigs
        };

        score
    }

    fn is_final(&self, signature: &Signature) -> bool {
        let votes = self.weights.signature_weight(signature)
            .unwrap_or_else(|| panic!("Missing weights for signature: {:?}", signature));

        trace!("is_final(): votes={}, final={}", votes, votes >= self.threshold);
        votes >= self.threshold
    }
}
