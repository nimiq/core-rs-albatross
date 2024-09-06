use std::{fmt::Formatter, sync::Arc, time::Duration};

use async_trait::async_trait;
use futures::{
    future::{BoxFuture, FutureExt},
    stream::StreamExt,
};
use nimiq_bls::PublicKey;
use nimiq_collections::bitset::BitSet;
use nimiq_handel::{
    aggregation::Aggregation,
    config::Config,
    contribution::{AggregatableContribution, ContributionError},
    evaluator::WeightedVote,
    identity::{Identity, IdentityRegistry, WeightRegistry},
    network::Network,
    partitioner::BinomialPartitioner,
    protocol,
    store::ReplaceStore,
    update::LevelUpdate,
    verifier::{VerificationResult, Verifier},
};
use nimiq_network_interface::{
    network::Network as NetworkInterface,
    request::{MessageMarker, OutboundRequestError, RequestCommon, RequestError},
};
use nimiq_network_mock::{MockHub, MockNetwork, MockPeerId};
use nimiq_test_log::test;
use nimiq_time::timeout;
use nimiq_utils::spawn;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

/// Dump Aggregate adding numbers.
#[derive(Clone, Debug, Serialize, Deserialize)]
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

// A dumb Registry
pub struct Registry {}

impl WeightRegistry for Registry {
    fn weight(&self, _id: usize) -> Option<usize> {
        Some(1)
    }
}

impl IdentityRegistry for Registry {
    fn public_key(&self, _id: usize) -> Option<PublicKey> {
        None
    }

    fn signers_identity(&self, slots: &BitSet) -> Identity {
        Identity::new(slots.clone())
    }
}

/// A dump Verifier who is happy with everything.
pub struct DumbVerifier {}

#[async_trait]
impl Verifier for DumbVerifier {
    type Contribution = Contribution;
    async fn verify(&self, _contribution: &Self::Contribution) -> VerificationResult {
        VerificationResult::Ok
    }
}

pub type Evaluator = WeightedVote<usize, Protocol>;

// The test protocol combining the other types.
pub struct Protocol {
    verifier: Arc<DumbVerifier>,
    partitioner: Arc<BinomialPartitioner>,
    evaluator: Arc<Evaluator>,
    store: Arc<RwLock<ReplaceStore<usize, Self>>>,
    registry: Arc<Registry>,
    node_id: usize,
}

impl Protocol {
    pub fn new(node_id: usize, num_ids: usize, threshold: usize) -> Self {
        let partitioner = Arc::new(BinomialPartitioner::new(node_id, num_ids));
        let registry = Arc::new(Registry {});
        let store = Arc::new(RwLock::new(ReplaceStore::<usize, Self>::new(
            partitioner.clone(),
        )));

        let evaluator = Arc::new(WeightedVote::new(
            store.clone(),
            Arc::clone(&registry),
            partitioner.clone(),
            threshold,
        ));

        Protocol {
            verifier: Arc::new(DumbVerifier {}),
            partitioner,
            evaluator,
            store,
            registry: Arc::new(Registry {}),
            node_id,
        }
    }
}

impl std::fmt::Debug for Protocol {
    fn fmt(&self, _f: &mut Formatter<'_>) -> Result<(), std::fmt::Error> {
        Ok(())
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(bound = "C: AggregatableContribution")]
struct SerializableLevelUpdate<C: AggregatableContribution> {
    aggregate: C,
    individual: Option<C>,
    level: u8,
}

impl<C: AggregatableContribution> SerializableLevelUpdate<C> {
    pub fn into_level_update(self, origin: u16) -> LevelUpdate<C> {
        LevelUpdate {
            aggregate: self.aggregate,
            individual: self.individual,
            level: self.level,
            origin,
        }
    }
}

impl<C: AggregatableContribution> From<LevelUpdate<C>> for SerializableLevelUpdate<C> {
    fn from(value: LevelUpdate<C>) -> Self {
        Self {
            aggregate: value.aggregate,
            individual: value.individual,
            level: value.level,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(bound = "C: AggregatableContribution")]
struct Update<C: AggregatableContribution>(pub SerializableLevelUpdate<C>);

impl<C: AggregatableContribution + 'static> RequestCommon for Update<C> {
    type Kind = MessageMarker;
    const TYPE_ID: u16 = 0;
    const MAX_REQUESTS: u32 = 100;
    const TIME_WINDOW: Duration = Duration::from_millis(500);
    type Response = ();
}

impl protocol::Protocol<usize> for Protocol {
    type Contribution = Contribution;
    type Verifier = DumbVerifier;
    type Registry = Registry;
    type Partitioner = BinomialPartitioner;
    type Store = ReplaceStore<usize, Self>;
    type Evaluator = WeightedVote<usize, Self>;

    fn verifier(&self) -> Arc<Self::Verifier> {
        self.verifier.clone()
    }
    fn registry(&self) -> Arc<Self::Registry> {
        self.registry.clone()
    }
    fn store(&self) -> Arc<RwLock<Self::Store>> {
        self.store.clone()
    }
    fn evaluator(&self) -> Arc<Self::Evaluator> {
        self.evaluator.clone()
    }
    fn partitioner(&self) -> Arc<Self::Partitioner> {
        self.partitioner.clone()
    }
    fn identify(&self) -> usize {
        self.node_id
    }
    fn node_id(&self) -> usize {
        self.node_id
    }
}

struct NetworkWrapper<N: NetworkInterface<PeerId = MockPeerId>>(Arc<N>);

impl<N: NetworkInterface<PeerId = MockPeerId>> Network for NetworkWrapper<N> {
    type Contribution = Contribution;
    type Error = RequestError;

    fn send_update(
        &self,
        node_id: u16,
        update: LevelUpdate<Self::Contribution>,
    ) -> BoxFuture<'static, Result<(), Self::Error>> {
        let update = Update(update.into());
        let network = Arc::clone(&self.0);
        async move {
            let peer_id = MockPeerId(node_id as u64);

            if network.has_peer(peer_id) {
                let result = network.message(update, peer_id).await;
                if let Err(err) = &result {
                    log::error!("Error sending request to {}: {:?}", node_id, err);
                }
                result
            } else {
                let this_id = network.get_local_peer_id();
                log::error!(?this_id, node_id, ?update, "Peer does not exist",);
                Err(OutboundRequestError::DialFailure.into())
            }
        }
        .boxed()
    }
}

#[test(tokio::test)]
async fn handel_aggregation() {
    let config = Config {
        update_interval: Duration::from_millis(500),
        level_timeout: Duration::from_millis(500),
        peer_count: 1,
    };

    let stopped = Arc::new(RwLock::new(false));

    let mut hub = MockHub::default();

    let num_contributors: usize = 12;
    log::info!(num_contributors, "Running with");

    // The final value needs to be the sum of all contributions.
    // For instance for `contributor_num = 7: 8 + 7 + 6 + 5 + 4 + 3 + 2 + 1 = 36`
    let mut expected_aggregation_value = 0u64;
    for i in 0..num_contributors {
        expected_aggregation_value += i as u64 + 1u64;
    }

    let (sender, mut receiver) = mpsc::channel(num_contributors);

    let mut networks: Vec<Arc<MockNetwork>> = vec![];
    // Initialize `num_contributors` networks and Handel Aggregations. Connect all the networks with
    // each other.
    for id in 0..num_contributors {
        // Create a network with id = `id`
        let net = Arc::new(hub.new_network_with_address(id as u64));
        // Create a protocol with `num_contributors` peers and set its id to `id`. Require
        // `num_contributors` contributions, meaning all contributions need to be aggregated.
        let protocol = Protocol::new(id, num_contributors, num_contributors);
        // The sole contributor is this node.
        let mut contributors = BitSet::new();
        contributors.insert(id);

        // Create a contribution for this node with a value of `id + 1`.
        // This is to ensure that no node has value 0 which doesn't show up in addition.
        let contribution = Contribution {
            value: id as u64 + 1,
            contributors,
        };

        // Connect the network to all already existing networks.
        for network in &networks {
            net.dial_mock(network);
        }

        // Remember the network so that subsequently created networks can connect to it.
        networks.push(net.clone());

        // Spawn a task for this Handel Aggregation and Network instance.
        let mut aggregation = Aggregation::new(
            protocol,
            config.clone(),
            contribution,
            net.receive_messages::<Update<Contribution>>()
                .map(move |msg| msg.0.0.into_level_update(msg.1.0 as u16))
                .boxed(),
            NetworkWrapper(net),
        );

        let stopped = stopped.clone();
        let sender = sender.clone();
        spawn(async move {
            loop {
                // Use a timeout here to eventually check the `stopped` flag if the aggregation
                // stream is not producing any more items.
                let item = timeout(Duration::from_secs(1), aggregation.next()).await;
                if let Ok(Some(contribution)) = item {
                    if contribution.num_contributors() == num_contributors {
                        assert_eq!(contribution.value, expected_aggregation_value);
                        sender.send(()).await.expect("Send should never fail");
                    }
                }

                if *stopped.read() {
                    return;
                }
            }
        });
    }

    // Check that all aggregations completed.
    let mut num_finished = 0usize;
    loop {
        let _ = timeout(Duration::from_secs(5), receiver.recv())
            .await
            .expect("An aggregation took too long to return");
        num_finished += 1;
        log::info!(num_finished, "Finished count");
        if num_finished == num_contributors {
            break;
        }
    }

    *stopped.write() = true;

    return;

    // // after we have the final aggregate create a new instance and have it (without any other instances)
    // // return a fully aggregated contribution and terminate.
    // // Same as before
    // // let net = Arc::new(hub.new_network_with_address(contributor_num as u64));
    // let protocol = Protocol::new(num_contributors, num_contributors + 1, num_contributors + 1);
    // let mut contributors = BitSet::new();
    // contributors.insert(num_contributors);
    // let contribution = Contribution {
    //     value: num_contributors as u64 + 1u64,
    //     contributors,
    // };
    // for network in &networks {
    //     net.dial_mock(network);
    // }
    //
    // // instead of spawning the aggregation task await its result here.
    // let mut aggregation = Aggregation::new(
    //     protocol,
    //     config.clone(),
    //     contribution,
    //     Box::pin(
    //         net.receive_messages::<Update<Contribution>>()
    //             .map(move |msg| msg.0 .0),
    //     ),
    //     NetworkWrapper(net),
    // );
    //
    // // first poll will add the nodes individual contribution and send its LevelUpdate
    // // which should be responded to with a full aggregation
    // let _ = aggregation.next().await;
    // // Second poll must return a full contribution
    // let last_aggregate = aggregation.next().await;
    //
    // // An aggregation needs to be present
    // assert!(last_aggregate.is_some(), "Nothing was aggregated!");
    //
    // let last_aggregate = last_aggregate.unwrap();
    //
    // // All nodes need to contribute
    // assert_eq!(
    //     last_aggregate.num_contributors(),
    //     num_contributors + 1,
    //     "Not all contributions are present: {:?}",
    //     last_aggregate,
    // );
    //
    // // the final value needs to be the sum of all contributions: `exp_agg_value`
    // assert_eq!(
    //     last_aggregate.value, expected_aggregation_value,
    //     "Wrong aggregation result",
    // );
    //
    // *stopped.write() = true;
}

// additional tests:
// it_sends_periodic_updates
// it_activates_levels
