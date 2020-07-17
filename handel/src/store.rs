use std::collections::BTreeMap;
use std::sync::Arc;

use collections::bitset::BitSet;

use crate::contribution::AggregatableContribution;
use crate::partitioner::Partitioner;

pub trait ContributionStore {
    type Contribution: AggregatableContribution;
    /// Put `signature` into the store for level `level`.
    fn put(&mut self, signature: Self::Contribution, level: usize);

    /// Return the number of the current best level.
    fn best_level(&self) -> usize;

    /// Check whether we have an individual signature from a certain `peer_id`.
    fn individual_received(&self, peer_id: usize) -> bool;

    /// Return a `BitSet` of the verified individual signatures we have for a given `level`.
    fn individual_verified(&self, level: usize) -> &BitSet;

    /// Returns a `BTreeMap` of the individual signatures for the given `level`.
    ///
    /// Panics if `level` is invalid.
    fn individual_signature(&self, level: usize, peer_id: usize) -> Option<&Self::Contribution>;

    /// Returns the best multi-signature for the given `level`.
    fn best(&self, level: usize) -> Option<&Self::Contribution>;

    /// Returns the best combined multi-signature for all levels upto `level`.
    fn combined(&self, level: usize) -> Option<Self::Contribution>;
}

#[derive(Clone, Debug)]
pub struct ReplaceStore<P: Partitioner, C: AggregatableContribution> {
    partitioner: Arc<P>,

    best_level: usize,

    /// BitSet that contains the IDs of all individual contributions we already received
    individual_received: BitSet,

    /// BitSets for all the individual contributions that we already verified
    /// level -> peer_id -> bool
    individual_verified: Vec<BitSet>,

    /// All individual contributions
    /// level -> peer_id -> Contribution
    individual_signatures: Vec<BTreeMap<usize, C>>,

    /// The best Contribution at each level
    best_contribution: BTreeMap<usize, C>,
}

impl<P: Partitioner, C: AggregatableContribution> ReplaceStore<P, C> {
    pub fn new(partitioner: Arc<P>) -> Self {
        let n = partitioner.size();
        let mut individual_verified = Vec::with_capacity(partitioner.levels());
        let mut individual_signatures = Vec::with_capacity(partitioner.levels());
        for level in 0..partitioner.levels() {
            individual_verified.push(BitSet::with_capacity(partitioner.level_size(level)));
            individual_signatures.push(BTreeMap::new())
        }

        Self {
            partitioner,
            best_level: 0,
            individual_received: BitSet::with_capacity(n),
            individual_verified,
            individual_signatures,
            best_contribution: BTreeMap::new(),
        }
    }

    fn check_merge(&self, contribution: &C, level: usize) -> Option<C> {
        if let Some(best_contribution) = self.best_contribution.get(&level) {
            // try to combine
            let mut contribution = contribution.clone();

            // we can ignore the error, if it's not possible to merge we continue
            contribution
                .combine(best_contribution)
                .unwrap_or_else(|e| trace!("check_merge: combining contributions failed: {}", e));

            let individual_verified = self.individual_verified.get(level).unwrap_or_else(|| {
                panic!(
                    "Individual verified contributions BitSet is missing for level {}",
                    level
                )
            });

            // the bits set here are verified individual signatures that can be added to `contribution`
            let complements =
                &(&contribution.contributors() & individual_verified) ^ individual_verified;

            // check that if we combine we get a better signature
            if complements.len() + contribution.num_contributors()
                <= best_contribution.num_contributors()
            {
                // XXX .weight()?
                // doesn't get better
                None
            } else {
                // put in the individual signatures
                for id in complements.iter() {
                    // get individual signature
                    let individual = self
                        .individual_signatures
                        .get(level)
                        .unwrap_or_else(|| {
                            panic!("Individual contribution missing for level {}", level)
                        })
                        .get(&id)
                        .unwrap_or_else(|| {
                            panic!(
                                "Individual contributioon {} missing for level {}",
                                id, level
                            )
                        });
                    // merge individual signature into multisig
                    // assert_eq!(id, individual.signer);
                    assert!(
                        individual.num_contributors() == 1
                            && individual.contributors().contains(id)
                    );
                    contribution.combine(individual).unwrap_or_else(|e| {
                        panic!(
                            "Individual contribution from id={} can't be added to aggregate contributions: {}",
                            id, e
                        )
                    });
                }

                Some(contribution)
            }
        } else {
            Some(contribution.clone())
        }
    }
}

impl<P: Partitioner, C: AggregatableContribution> ContributionStore for ReplaceStore<P, C> {
    type Contribution = C;

    fn put(&mut self, contribution: Self::Contribution, level: usize) {
        trace!(
            "Putting signature into store (level {}): {:?}",
            level,
            contribution,
        );
        // let multisig = match signature {
        //     Signature::Individual(individual) => {
        //         let multisig = individual.as_multisig();
        //         self.individual_verified
        //             .get_mut(level)
        //             .unwrap_or_else(|| panic!("Missing level {}", level))
        //             .insert(individual.signer);
        //         self.individual_signatures
        //             .get_mut(level)
        //             .unwrap_or_else(|| panic!("Missing level {}", level))
        //             .insert(individual.signer, individual);
        //         multisig
        //     }
        //     Signature::Multi(multisig) => multisig,
        // };

        if let Some(best_contribution) = self.check_merge(&contribution, level) {
            trace!(
                "best_contribution = {:?} (level {})",
                best_contribution,
                level
            );
            self.best_contribution.insert(level, best_contribution);
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
        self.individual_verified
            .get(level)
            .unwrap_or_else(|| panic!("Invalid level: {}", level))
    }

    fn individual_signature(&self, level: usize, peer_id: usize) -> Option<&Self::Contribution> {
        self.individual_signatures
            .get(level)
            .unwrap_or_else(|| panic!("Invalid level: {}", level))
            .get(&peer_id)
    }

    fn best(&self, level: usize) -> Option<&Self::Contribution> {
        self.best_contribution.get(&level)
    }

    fn combined(&self, mut level: usize) -> Option<Self::Contribution> {
        // TODO: Cache this?
        trace!("Creating combined signature for level {}", level);

        let mut signatures = Vec::new();
        for (_, signature) in self.best_contribution.range(0..=level) {
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
