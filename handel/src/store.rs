use std::sync::Arc;
use std::collections::BTreeMap;

use collections::bitset::BitSet;

use crate::multisig::{Signature, IndividualSignature, MultiSignature};
use crate::partitioner::Partitioner;


pub trait SignatureStore {
    /// Put `signature` into the store for level `level`.
    fn put(&mut self, signature: Signature, level: usize);

    /// Return the number of the current best level.
    fn best_level(&self) -> usize;

    /// Check whether we have an individual signature from a certain `peer_id`.
    fn individual_received(&self, peer_id: usize) -> bool;

    /// Return a `BitSet` of the verified individual signatures we have for a given `level`.
    fn individual_verified(&self, level: usize) -> &BitSet;

    /// Returns a `BTreeMap` of the individual signatures for the given `level`.
    ///
    /// Panics if `level` is invalid.
    fn individual_signature(&self, level: usize, peer_id: usize) -> Option<&IndividualSignature>;

    /// Returns the best multi-signature for the given `level`.
    fn best(&self, level: usize) -> Option<&MultiSignature>;

    /// Returns the best combined multi-signature for all levels upto `level`.
    fn combined(&self, level: usize) -> Option<MultiSignature>;
}


#[derive(Clone, Debug)]
pub struct ReplaceStore<P: Partitioner> {
    partitioner: Arc<P>,

    best_level: usize,

    /// BitSet that contains the IDs of all individual signatures we already received
    individual_received: BitSet,

    /// BitSets for all the individual signatures that we already verified
    /// level -> peer_id -> bool
    individual_verified: Vec<BitSet>,

    /// All individual signatures
    /// level -> peer_id -> Signature
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
                .unwrap_or_else(|e| trace!("check_merge: combining multisigs failed: {}", e));

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
                    let individual = self.individual_signatures.get(level)
                        .unwrap_or_else(|| panic!("Individual signatures missing for level {}", level))
                        .get(&id)
                        .unwrap_or_else(|| panic!("Individual signature {} missing for level {}", id, level));
                    // merge individual signature into multisig
                    assert_eq!(id, individual.signer);
                    multisig.add_individual(individual)
                        .unwrap_or_else(|e| panic!("Individual signature from id={} can't be added to multisig: {}", id, e));
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
    fn put(&mut self, signature: Signature, level: usize) {
        trace!("Putting signature into store (level {}): {:?}", level, signature);
        let multisig = match signature {
            Signature::Individual(individual) => {
                let multisig = individual.as_multisig();
                self.individual_verified.get_mut(level)
                    .unwrap_or_else(|| panic!("Missing level {}", level))
                    .insert(individual.signer);
                self.individual_signatures.get_mut(level)
                    .unwrap_or_else(|| panic!("Missing level {}", level))
                    .insert(individual.signer, individual);
                multisig
            },
            Signature::Multi(multisig) => multisig,
        };

        if let Some(best_multisig) = self.check_merge(&multisig, level) {
            trace!("best_multisig = {:?} (level {})", best_multisig, level);
            self.multisig_best.insert(level, best_multisig);
            if level > self.best_level {
                trace!("best level is now {}", level);
                self.best_level = level;
            }
        }
    }

    fn best_level(&self) -> usize {
        self.best_level
    }

    fn individual_received(&self, peer_id: usize) -> bool {
        self.individual_received.contains(peer_id)
    }

    fn individual_verified(&self, level: usize) -> &BitSet {
        self.individual_verified.get(level)
            .unwrap_or_else(|| panic!("Invalid level: {}", level))
    }

    fn individual_signature(&self, level: usize, peer_id: usize) -> Option<&IndividualSignature> {
        self.individual_signatures.get(level)
            .unwrap_or_else(|| panic!("Invalid level: {}", level))
            .get(&peer_id)
    }

    fn best(&self, level: usize) -> Option<&MultiSignature> {
        self.multisig_best.get(&level)
    }

    fn combined(&self, mut level: usize) -> Option<MultiSignature> {
        // TODO: Cache this?
        trace!("Creating combined signature for level {}", level);

        let mut signatures = Vec::new();
        for (_, signature) in self.multisig_best.range(0 ..= level) {
            trace!("collect: {:?}", signature);
            signatures.push(signature)
        }
        trace!("Collected {} signatures", signatures.len());

        // ???
        if level < self.partitioner.levels() - 1 {
            level += 1;
        }

        let combined = self.partitioner.combine(signatures, level);
        trace!("Combined signature for level {}: {:?}", level, combined);
        combined
    }
}
