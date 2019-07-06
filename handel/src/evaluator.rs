use std::sync::Arc;

use parking_lot::RwLock;

use collections::bitset::BitSet;

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
    fn evaluate(&self, signature: &Signature, level: usize) -> usize {
        /*let to_receive = self.partitioner.size(level);
        let best_signature = self.signature_store.best_multisig(&level);

        if let Some(best_signature) = best_signature {
            // check if the best signature for that level is already complete
            if to_receive == best_signature.len() {
                //debug!("Best signature already complete");
                return 0;
            }

            // check if the best signature is better than the new one
            if best_signature.signers.is_superset(&signature.signers().collect::<BitSet>()) {
                //debug!("Best signature is better");
                return 0;
            }
        }

        let with_individuals = &multisig.signers
            | self.individual_verified.get(level)
            .unwrap_or_else(|| panic!("Missing level {}", level));

        let (new_total, added_sigs, combined_sigs) = if let Some(best_signature) = best_signature {
            if multisig.signers.intersection_size(&best_signature.signers) > 0 {
                // can't merge
                let new_total = with_individuals.len();
                (new_total, new_total.saturating_sub(best_signature.len()), new_total - multisig.len())
            }
            else {
                let final_sig = &with_individuals | &best_signature.signers;
                let new_total = final_sig.len();
                let combined_sigs = (final_sig ^ (&best_signature.signers | &multisig.signers)).len();
                (new_total, new_total - best_signature.len(), combined_sigs)
            }
        }
        else {
            // best is the new signature with the individual signatures
            let new_total = with_individuals.len();
            (new_total, new_total, new_total - multisig.len())
        };

        //debug!("new_total={}, added_sigs={}, combined_sigs={}", new_total, added_sigs, combined_sigs);

        if added_sigs == 0 {
            debug!("No new signatures added");
            // XXX return 1 for an individual signature
            if multisig.len() == 1 { 1 } else { 0 }
        }
        else if new_total == to_receive {
            1000000 - level * 10 - combined_sigs
        }
        else {
            100000 - level * 100 + added_sigs * 10 - combined_sigs
        }*/

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
    signature_store: Arc<RwLock<S>>,
    identity_registry: Arc<I>,
    partitioner: Arc<P>,
    threshold: usize
}

impl<S: SignatureStore, I: WeightRegistry, P: Partitioner> WeightedVote<S, I, P> {
    pub fn new(signature_store: Arc<RwLock<S>>, identity_registry: Arc<I>, partitioner: Arc<P>, threshold: usize) -> Self {
        Self {
            signature_store,
            identity_registry,
            partitioner,
            threshold,
        }
    }
}

impl<S: SignatureStore, I: WeightRegistry, P: Partitioner> Evaluator for WeightedVote<S, I, P> {
    fn evaluate(&self, signature: &Signature, level: usize) -> usize {
        unimplemented!()
    }

    fn is_final(&self, signature: &Signature) -> bool {
        if let Some(votes) = self.identity_registry.signature_weight(signature) {
            votes >= self.threshold
        }
        else {
            false
        }
    }
}
