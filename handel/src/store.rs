use std::{collections::BTreeMap, sync::Arc};

use crate::{
    contribution::AggregatableContribution,
    identity::{Identity, IdentityRegistry},
    partitioner::Partitioner,
    protocol::Protocol,
    Identifier,
};

/// Trait that needs to be implemented to support the storage of contributions
/// and the selection of the best contributions seen.
pub trait ContributionStore<TId, TProtocol>
where
    TId: Identifier,
    TProtocol: Protocol<TId>,
    Self: Send + Sync,
{
    /// Put `signature` into the store for level `level`.
    /// In the case of individual signatures they must not already exist.
    fn put(
        &mut self,
        contribution: TProtocol::Contribution,
        level: usize,
        registry: Arc<TProtocol::Registry>,
        identifier: TId,
    );

    /// Return the number of the current best level.
    fn best_level(&self) -> usize;

    /// Return a `BitSet` of the verified individual signatures we have for a given `level`.
    ///
    /// Panics if `level` is invalid.
    fn individual_verified(&self, level: usize) -> &Identity;

    /// Returns a `BTreeMap` of the individual signatures for the given `level`.
    ///
    /// Panics if `level` is invalid.
    fn individual_signature(
        &self,
        level: usize,
        peer_id: &Identity,
    ) -> Option<&TProtocol::Contribution>;

    /// Returns the best multi-signature for the given `level`.
    ///
    /// Panics if `level` is invalid.
    fn best(&self, level: usize) -> Option<&TProtocol::Contribution>;

    /// Returns the best combined multi-signature for all levels up to `level`.
    fn combined(&self, level: usize) -> Option<TProtocol::Contribution>;
}

#[derive(Clone, Debug)]
/// An implementation of the `ContributionStore` trait
pub struct ReplaceStore<TId, TProtocol>
where
    TId: Identifier,
    TProtocol: Protocol<TId>,
{
    /// The Partitioner used to create the Aggregation Tree.
    partitioner: Arc<TProtocol::Partitioner>,

    /// The currently best level
    best_level: usize,

    /// BitSets for all the individual contributions that we already verified
    /// level -> peer_id -> bool
    individual_verified: Vec<Identity>,

    /// All individual contributions
    /// level -> peer_id -> Contribution
    individual_contributions: Vec<BTreeMap<Identity, TProtocol::Contribution>>,

    /// The best Contribution at each level
    best_contribution: BTreeMap<usize, (TProtocol::Contribution, Identity)>,
}

impl<TId, TProtocol> ReplaceStore<TId, TProtocol>
where
    TId: Identifier,
    TProtocol: Protocol<TId>,
{
    /// Create a new Replace Store using the Partitioner `partitioner`.
    pub fn new(partitioner: Arc<TProtocol::Partitioner>) -> Self {
        Self {
            individual_verified: vec![Identity::default(); partitioner.levels()],
            individual_contributions: vec![BTreeMap::default(); partitioner.levels()],
            partitioner,
            best_level: 0,
            best_contribution: BTreeMap::new(),
        }
    }

    /// Attempts to merge the given contribution with the best contribution on the given level.
    ///
    /// If the signatures cannot be merged all available individual signatures will be merged into the given
    /// signature if that would produce a stronger signature than the current best for the given level.
    ///
    /// ## Returns
    /// * The combined contribution produced as well as the identity present in the contribution if there was an improvement.
    /// * None otherwise.
    fn check_merge(
        &self,
        mut contribution: TProtocol::Contribution,
        registry: Arc<TProtocol::Registry>,
        level: usize,
        identifier: TId,
    ) -> Option<(TProtocol::Contribution, Identity)> {
        let best_contribution = self.best_contribution.get(&level);

        let Some((best_contribution, best_contributors)) = best_contribution else {
            // This is normal whenever the first signature for a level is processed.
            trace!(
                id = %identifier,
                level,
                contributors = %contribution.contributors(),
                "Level was empty",
            );
            let contributors = registry.signers_identity(&contribution.contributors());
            return Some((contribution, contributors));
        };

        trace!(
            id = %identifier,
            level,
            ?best_contribution,
            "Current best for level"
        );

        // Try to combine the contributions.
        // If combining fails, it is due to the contributions having an overlap.
        // One may be the superset of the other which makes it the strictly better set.
        // If that is the case the better can be immediately returned.
        // Otherwise, individual signatures must still be checked.
        let mut contributors = if let Err(e) = contribution.combine(best_contribution) {
            // The contributors of contribution represented as an Identity.
            let contributors = registry.signers_identity(&contribution.contributors());

            // Merging failed. Check if contribution is a superset of `best_contribution`.
            if contributors.is_superset_of(best_contributors) {
                trace!(
                    id = %identifier,
                    level,
                    ?contribution,
                    ?best_contribution,
                    "Contribution is superset of current best, replacing",
                );
                return Some((contribution, contributors));
            } else {
                trace!(
                    id = %identifier,
                    level,
                    ?contribution,
                    ?best_contribution,
                    error = ?e,
                    "Combining contributions failed",
                );
                contributors
            }
        } else {
            // Combining worked meaning the contributions did not overlap. Since individuals should already
            // have been present in the previous best this is the best signature available.
            let contributors = registry.signers_identity(&contribution.contributors());
            return Some((contribution, contributors));
        };

        // Individual signatures of identities which this node has that are already verified.
        let individual_verified = self.individual_verified.get(level).unwrap_or_else(|| {
            panic!("Individual verified contributions is missing for level {level}")
        });

        // The identity here describes which verified individual signatures that can be added to `contribution`
        let complements = contributors.complement(individual_verified);

        // Check that if we combine we get a better signature
        if complements.len() + contributors.len() <= best_contributors.len() {
            // This should not be observed really as the evaluator should filter signatures which cannot provide
            // improvements out.
            trace!(
                id = %identifier,
                "No improvement possible",
            );
            return None;
        }

        // Put in the individual signatures
        for id in complements.iter() {
            // Get individual signature
            let individual = &self.individual_contributions[level][&id];

            // Merge individual signature into multisig
            contribution
                .combine(individual)
                .unwrap_or_else(|e| panic!("Individual contribution from id={:?} can't be added to aggregate contributions: {:?}", id, e));
            contributors.combine(&id, false);
        }

        Some((contribution, contributors))
    }
}

impl<TId, TProtocol> ContributionStore<TId, TProtocol> for ReplaceStore<TId, TProtocol>
where
    TId: Identifier,
    TProtocol: Protocol<TId>,
{
    fn put(
        &mut self,
        contribution: TProtocol::Contribution,
        level: usize,
        registry: Arc<TProtocol::Registry>,
        identifier: TId,
    ) {
        let identity = registry.signers_identity(&contribution.contributors());
        if identity.is_empty() {
            return;
        }

        // In case of individual contributions additional tracking is required.
        if identity.len() == 1 {
            // These will panic if the level does not exist or if the individual signature already exists.
            // If it does the evaluator has a bug, as existing signatures should be scored 0.
            self.individual_verified[level].combine(&identity, false);
            assert!(self.individual_contributions[level]
                .insert(identity, contribution.clone())
                .is_none());
        }

        if let Some((best_contribution, best_identity)) = self.check_merge(
            contribution,
            Arc::clone(&registry),
            level,
            identifier.clone(),
        ) {
            self.best_contribution
                .insert(level, (best_contribution, best_identity));
            if level > self.best_level {
                trace!(
                    id = %identifier,
                    level,
                    "Best level is now",
                );
                self.best_level = level;
            }
        }
    }

    fn best_level(&self) -> usize {
        self.best_level
    }

    fn individual_verified(&self, level: usize) -> &Identity {
        self.individual_verified
            .get(level)
            .unwrap_or_else(|| panic!("Invalid level: {level}"))
    }

    fn individual_signature(
        &self,
        level: usize,
        identity: &Identity,
    ) -> Option<&TProtocol::Contribution> {
        if identity.len() != 1 {
            return None;
        }

        self.individual_contributions
            .get(level)
            .unwrap_or_else(|| panic!("Invalid level: {level}"))
            .get(identity)
    }

    fn best(&self, level: usize) -> Option<&TProtocol::Contribution> {
        self.best_contribution.get(&level).map(|(c, _)| c)
    }

    fn combined(&self, level: usize) -> Option<TProtocol::Contribution> {
        let mut signatures = Vec::new();
        for (_, (signature, _)) in self.best_contribution.range(0..=level) {
            signatures.push(signature)
        }

        self.partitioner.combine(signatures, level)
    }
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;
    use nimiq_collections::BitSet;
    use nimiq_test_log::test;
    use parking_lot::RwLock;
    use rand::Rng;
    use serde::{Deserialize, Serialize};

    use super::*;
    use crate::{
        contribution::ContributionError,
        evaluator::WeightedVote,
        identity::WeightRegistry,
        partitioner::BinomialPartitioner,
        verifier::{VerificationResult, Verifier},
    };

    /// Dummy Registry, unused except for TestProtocol type definition
    pub struct TestRegistry {}
    impl WeightRegistry for TestRegistry {
        fn weight(&self, _id: usize) -> Option<usize> {
            None
        }
    }
    impl IdentityRegistry for TestRegistry {
        fn public_key(&self, _id: usize) -> Option<nimiq_bls::PublicKey> {
            None
        }
        fn signers_identity(&self, slots: &BitSet) -> Identity {
            Identity::new(slots.clone())
        }
    }
    pub struct TestVerifier {}
    #[async_trait]
    impl Verifier for TestVerifier {
        type Contribution = Contribution;
        async fn verify(&self, _contribution: &Self::Contribution) -> VerificationResult {
            VerificationResult::Ok
        }
    }

    /// Dummy Protocol. Unused, except for Generic
    pub struct TestProtocol {
        evaluator: Arc<<Self as Protocol<u8>>::Evaluator>,
        partitioner: Arc<<Self as Protocol<u8>>::Partitioner>,
        registry: Arc<<Self as Protocol<u8>>::Registry>,
        verifier: Arc<<Self as Protocol<u8>>::Verifier>,
        store: Arc<RwLock<<Self as Protocol<u8>>::Store>>,
    }
    impl Protocol<u8> for TestProtocol {
        type Contribution = Contribution;
        type Evaluator = WeightedVote<u8, Self>;
        type Partitioner = BinomialPartitioner;
        type Registry = TestRegistry;
        type Verifier = TestVerifier;
        type Store = ReplaceStore<u8, Self>;
        fn identify(&self) -> u8 {
            0
        }
        fn node_id(&self) -> usize {
            0
        }
        fn evaluator(&self) -> Arc<Self::Evaluator> {
            Arc::clone(&self.evaluator)
        }
        fn partitioner(&self) -> Arc<Self::Partitioner> {
            Arc::clone(&self.partitioner)
        }
        fn registry(&self) -> Arc<Self::Registry> {
            Arc::clone(&self.registry)
        }
        fn verifier(&self) -> Arc<Self::Verifier> {
            Arc::clone(&self.verifier)
        }
        fn store(&self) -> Arc<RwLock<Self::Store>> {
            Arc::clone(&self.store)
        }
    }

    /// Dumb Aggregate adding numbers.
    #[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
    pub struct Contribution {
        value: u64,
        contributors: BitSet,
    }

    impl AggregatableContribution for Contribution {
        fn contributors(&self) -> BitSet {
            self.contributors.clone()
        }

        fn combine(&mut self, other_contribution: &Self) -> Result<(), ContributionError> {
            let overlap = &self.contributors & &other_contribution.contributors;
            // only contributions without any overlap can be combined.
            if overlap.is_empty() {
                // the combined value is the addition of the 2 individual values
                self.value += other_contribution.value;
                // the contributors of the resulting contribution are the combined sets of both individual contributions.
                self.contributors = &self.contributors | &other_contribution.contributors;
                Ok(())
            } else {
                Err(ContributionError::Overlapping(overlap))
            }
        }
    }

    #[test]
    fn it_can_combine_contributions() {
        let mut rng = rand::thread_rng();
        let num_ids = rng.gen_range(8..512);

        let node_id = rng.gen_range(0..num_ids);

        log::debug!(num_ids, node_id);

        // Create the partitions
        let partitioner = Arc::new(BinomialPartitioner::new(node_id, num_ids));

        let store = Arc::new(RwLock::new(ReplaceStore::<u8, TestProtocol>::new(
            partitioner.clone(),
        )));

        // Define a level that is going to be used for the contributions
        let level = rng.gen_range(0..partitioner.levels()) as usize;

        // Create the first contribution
        let mut first_contributors = BitSet::new();
        first_contributors.insert(0);

        let first_contribution = Contribution {
            value: 1,
            contributors: first_contributors,
        };

        store.write().put(
            first_contribution.clone(),
            level,
            Arc::new(TestRegistry {}),
            0,
        );

        {
            let store = store.read();
            let current_best_contribution = store.best(level);

            if let Some(best) = current_best_contribution {
                assert_eq!(first_contribution, *best);
                log::debug!(?best, "Current best contribution");
            }
        }

        // Create a second contribution, using a different identity and a different contributor
        let mut second_contributors = BitSet::new();
        second_contributors.insert(1);

        let second_contribution = Contribution {
            value: 10,
            contributors: second_contributors,
        };

        store.write().put(
            second_contribution.clone(),
            level,
            Arc::new(TestRegistry {}),
            0,
        );

        {
            let store = store.read();
            let current_best_contribution = store.best(level);

            if let Some(best) = current_best_contribution {
                // Now the best contribution should be the aggregated one
                let mut contributors = BitSet::new();
                contributors.insert(0);
                contributors.insert(1);

                let aggregated_contribution = Contribution {
                    value: 11,
                    contributors,
                };

                log::debug!(?best, "Current best contribution");
                assert_eq!(aggregated_contribution, *best);
            }
        }
    }

    #[test]
    fn it_doesnt_get_better_contribution() {
        let mut rng = rand::thread_rng();
        let num_ids = rng.gen_range(8..512);

        let node_id = rng.gen_range(0..num_ids);

        log::debug!(num_ids, node_id);

        // Create the partitions
        let partitioner = Arc::new(BinomialPartitioner::new(node_id, num_ids));

        let store = Arc::new(RwLock::new(ReplaceStore::<u8, TestProtocol>::new(
            partitioner.clone(),
        )));

        // Define a level that is going to be used for the contributions
        let level = rng.gen_range(0..partitioner.levels()) as usize;

        // Create the first contribution
        let mut first_contributors = BitSet::new();
        first_contributors.insert(1);
        first_contributors.insert(2);
        first_contributors.insert(3);
        first_contributors.insert(4);

        let first_contribution = Contribution {
            value: 1,
            contributors: first_contributors,
        };

        store.write().put(
            first_contribution.clone(),
            level,
            Arc::new(TestRegistry {}),
            0,
        );

        {
            let store = store.read();
            let best_contribution = store
                .best(level)
                .expect("We should have a best contribution");

            assert_eq!(first_contribution, *best_contribution);
            log::debug!(?best_contribution, "Current best contribution");
        }

        // Create a second contribution, using a different contributor set
        // (with no repeated contributors from the first contribution)
        let mut second_contributors = BitSet::new();
        second_contributors.insert(5);
        second_contributors.insert(6);

        let second_contribution = Contribution {
            value: 10,
            contributors: second_contributors,
        };

        store.write().put(
            second_contribution.clone(),
            level,
            Arc::new(TestRegistry {}),
            0,
        );

        {
            let store = store.read();
            let best_contribution = store
                .best(level)
                .expect("We should have a best contribution");

            // Now the best contribution should be the aggregated one
            let mut contributors = BitSet::new();
            contributors.insert(1);
            contributors.insert(2);
            contributors.insert(3);
            contributors.insert(4);
            contributors.insert(5);
            contributors.insert(6);

            let aggregated_contribution = Contribution {
                value: 11,
                contributors,
            };

            log::debug!(?best_contribution, "Current best contribution");
            assert_eq!(aggregated_contribution, *best_contribution);
        }

        // Now try to insert a contribution using some of the same previous contributors
        let mut third_contributors = BitSet::new();
        // Note that we are using a different contributor number
        third_contributors.insert(1);
        third_contributors.insert(7);

        let third_contribution = Contribution {
            value: 100,
            contributors: third_contributors,
        };

        store.write().put(
            third_contribution.clone(),
            level,
            Arc::new(TestRegistry {}),
            0,
        );

        {
            let store = store.read();
            let best_contribution = store
                .best(level)
                .expect("We should have a best contribution");

            // Now the best contribution is still the previous one,
            // since we don't have any new contributor

            let mut contributors = BitSet::new();
            contributors.insert(1);
            contributors.insert(2);
            contributors.insert(3);
            contributors.insert(4);
            contributors.insert(5);
            contributors.insert(6);

            let aggregated_contribution = Contribution {
                value: 11,
                contributors,
            };
            assert_eq!(aggregated_contribution, *best_contribution);

            log::debug!(?best_contribution, "Final best contribution");
        }
    }
}
