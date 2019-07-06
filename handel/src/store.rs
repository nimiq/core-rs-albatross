use std::sync::Arc;
use std::collections::BTreeMap;

use collections::bitset::BitSet;

use crate::multisig::{Signature, IndividualSignature, MultiSignature};
use crate::partitioner::Partitioner;


pub trait SignatureStore {
    fn evaluate(&self, signature: Signature, level: usize) -> usize;
    fn put(&mut self, signature: Signature, level: usize);
    fn best(&self, level: usize) -> Option<&MultiSignature>;
    fn combined(&self, level: usize) -> Option<MultiSignature>;
}


#[derive(Clone, Debug)]
pub struct ReplaceStore<P: Partitioner> {
    partitioner: Arc<P>,

    best_level: usize,

    /// BitSet that contains the IDs of all individual signatures we already received
    individual_received: BitSet,

    /// BitSets for all the individual signatures that we already verified
    /// level -> bitset
    individual_verified: Vec<BitSet>,

    /// All individual signatures
    /// level -> ID -> Signature
    individual_signatures: Vec<BTreeMap<usize, IndividualSignature>>,

    /// The best MultiSignature at each level
    multisig_best: BTreeMap<usize, MultiSignature>,
}

impl<P: Partitioner> ReplaceStore<P> {
    pub fn new(partitioner: Arc<P>) -> Self {
        let n = partitioner.size();
        let mut individual_verified = Vec::with_capacity(partitioner.levels());
        let mut individual_signatures = Vec::with_capacity(partitioner.levels());
        for level in 0 .. partitioner.levels() {
            individual_verified.push(BitSet::with_capacity(partitioner.level_size(level)));
            individual_signatures.push(BTreeMap::new())
        }

        Self {
            partitioner,
            best_level: 0,
            individual_received: BitSet::with_capacity(n),
            individual_verified,
            individual_signatures,
            multisig_best: BTreeMap::new(),
        }
    }

    fn check_merge(&self, multisig: &MultiSignature, level: usize) -> Option<MultiSignature> {
        if let Some(best_multisig) = self.multisig_best.get(&level) {
            // try to combine
            let mut multisig = multisig.clone();

            // we can ignore the error, if it's not possible to merge we continue
            multisig.add_multisig(best_multisig)
                .unwrap_or_else(|e| debug!("check_merge: combining multisigs failed: {}", e));

            let individual_verified = self.individual_verified.get(level)
                .unwrap_or_else(|| panic!("Individual verified signatures BitSet is missing for level {}", level));

            // the bits set here are verified individual signatures that can be added to `multisig`
            let complements = &(&multisig.signers & individual_verified) ^ individual_verified;

            // check that if we combine we get a better signature
            if complements.len() + multisig.len() <= best_multisig.len() {
                // doesn't get better
                None
            }
            else {
                // put in the individual signatures
                for id in complements.iter() {
                    // get individual signature
                    // TODO: Why do we need to store individual signatures per level?
                    let individual = self.individual_signatures.get(level)
                        .unwrap_or_else(|| panic!("Individual signatures missing for level {}", level))
                        .get(&id).unwrap_or_else(|| panic!("Individual signature with ID {} missing for level {}", id, level));

                    // merge individual signature into multisig
                    multisig.add_individual(individual)
                        .unwrap_or_else(|e| panic!("Individual signature form id={} can't be added to multisig: {}", id, e));
                }

                Some(multisig)
            }
        }
        else {
            Some(multisig.clone())
        }
    }
}

impl<P: Partitioner> SignatureStore for ReplaceStore<P> {
    fn evaluate(&self, signature: Signature, level: usize) -> usize {
        unimplemented!()
    }

    fn put(&mut self, signature: Signature, level: usize) {
        let multisig = match signature {
            Signature::Individual(individual) => {
                let multisig = individual.as_multisig();
                self.individual_verified.get_mut(level)
                    .unwrap_or_else(|| panic!("Missing level {}", level))
                    .insert(individual.signer);
                self.individual_signatures.get_mut(level)
                    .unwrap_or_else(|| panic!("Missing level {}", level))
                    .insert(level, individual);
                multisig
            },
            Signature::Multi(multisig) => multisig,
        };

        if let Some(best_multisig) = self.check_merge(&multisig, level) {
            self.multisig_best.insert(level, best_multisig);
            if level > self.best_level {
                self.best_level = level;
            }
        }
    }

    fn best(&self, level: usize) -> Option<&MultiSignature> {
        self.multisig_best.get(&level)
    }

    fn combined(&self, mut level: usize) -> Option<MultiSignature> {
        let mut signatures = Vec::new();
        for (&i, signature) in self.multisig_best.range(0 ..= level) {
            if i > signatures.len()  {
                //warn!("MultiSignature missing for level {} to {}", signatures.len(), i - 1);
                return None;
            }
            signatures.push(signature)
        }

        // ???
        if level < self.partitioner.levels() - 1 {
            level += 1;
        }

        //debug!("Combining signatures for level {}: {:?}", level, signatures);
        self.partitioner.combine(signatures, level)
    }
}
