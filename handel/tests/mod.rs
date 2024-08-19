use std::{fmt::Formatter, sync::Arc, time::Duration};

use async_trait::async_trait;
use futures::{
    future::{BoxFuture, FutureExt},
    stream::StreamExt,
};
use instant::Instant;
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
    request::{MessageMarker, RequestCommon},
};
use nimiq_network_mock::{MockHub, MockNetwork, MockPeerId};
use nimiq_test_log::test;
use nimiq_utils::spawn;
use parking_lot::RwLock;
use rand::{thread_rng, Rng};
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

        let cloned_registry = Arc::clone(&registry);
        let evaluator = Arc::new(WeightedVote::new(
            store.clone(),
            Arc::clone(&registry),
            partitioner.clone(),
            move |c| cloned_registry.signature_weight(c).unwrap_or(0) >= threshold,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(bound = "C: AggregatableContribution")]
struct Update<C: AggregatableContribution>(pub LevelUpdate<C>);

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

    fn send_to(
        &self,
        (msg, recipient): (LevelUpdate<Self::Contribution>, u16),
    ) -> BoxFuture<'static, ()> {
        let update = Update(msg);
        let nw = Arc::clone(&self.0);
        async move {
            let peer_id = MockPeerId(recipient as u64);

            if nw.has_peer(peer_id) {
                if let Err(err) = nw.message(update, peer_id).await {
                    log::error!("Error sending request to {}: {:?}", recipient, err);
                }
            } else {
                let this_id = nw.get_local_peer_id();
                log::error!(?this_id, ?recipient, ?update, "Peer does not exist",);
            }
        }
        .boxed()
    }
}

#[test(tokio::test)]
async fn it_can_aggregate() {
    let config = Config {
        update_count: 1,
        update_interval: Duration::from_millis(500),
        timeout: Duration::from_millis(500),
        peer_count: 1,
    };

    let stopped = Arc::new(RwLock::new(false));

    let mut hub = MockHub::default();

    let mut rng = thread_rng();
    let contributor_num: usize = rng.gen_range(7..15);
    log::info!(contributor_num, "Running with");

    let (sender, mut receiver) = mpsc::channel(contributor_num);

    let mut networks: Vec<Arc<MockNetwork>> = vec![];
    // Initialize `contributor_num` networks and Handel Aggregations. Connect all the networks with each other.
    for id in 0..contributor_num {
        // Create a network with id = `id`
        let net = Arc::new(hub.new_network_with_address(id as u64));
        // Create a protocol with `contributor_num + 1` peers set its id to `id`. Require `contributor_num` contributions
        // meaning all contributions need to be aggregated with the additional node initialized after this for loop.
        let protocol = Protocol::new(id, contributor_num + 1, contributor_num + 1);
        // the sole contributor for soon to be created contribution is this node.
        let mut contributors = BitSet::new();
        contributors.insert(id);

        // create a contribution for this node with a value of `id + 1` (So no node has value 0 which doesn't show up in addition).
        let contribution = Contribution {
            value: id as u64 + 1u64,
            contributors,
        };
        // connect the network to all already existing networks.
        for network in &networks {
            net.dial_mock(network);
        }
        // remember the network so that subsequently created networks can connect to it.
        networks.push(net.clone());

        // spawn a task for this Handel Aggregation and Network instance.
        let mut aggregation = Aggregation::new(
            protocol,
            config.clone(),
            contribution,
            Box::pin(
                net.receive_messages::<Update<Contribution>>()
                    .map(move |msg| msg.0 .0),
            ),
            NetworkWrapper(net),
        );

        let r = stopped.clone();
        let s = sender.clone();
        spawn(async move {
            // have them just run until the aggregation is finished
            while let Some(contribution) = aggregation.next().await {
                if contribution.num_contributors() == contributor_num + 1 {
                    s.send(()).await.expect("Send should never fail");
                }

                if *r.read() {
                    return;
                }
            }
        });
    }

    // same as in the for loop, except we want to keep the handel instance and not spawn it.
    let net = Arc::new(hub.new_network_with_address(contributor_num as u64));
    let protocol = Protocol::new(contributor_num, contributor_num + 1, contributor_num + 1);
    let mut contributors = BitSet::new();
    contributors.insert(contributor_num);
    let contribution = Contribution {
        value: contributor_num as u64 + 1u64,
        contributors,
    };
    for network in &networks {
        net.dial_mock(network);
    }

    // instead of spawning the aggregation task await its result here.
    let mut aggregation = Aggregation::new(
        protocol,
        config.clone(),
        contribution,
        Box::pin(
            net.receive_messages::<Update<Contribution>>()
                .map(move |msg| msg.0 .0),
        ),
        NetworkWrapper(Arc::clone(&net)),
    );

    // aggregating should not take more than 300 ms per each 7 contributors
    let timeout_ms = 300u64 * (contributor_num / 7 + 1) as u64;

    let deadline = Instant::now()
        .checked_add(Duration::from_millis(timeout_ms))
        .unwrap();

    // The final value needs to be the sum of all contributions.
    // For instance for `contributor_num = 7: 8 + 7 + 6 + 5 + 4 + 3 + 2 + 1 = 36`
    let mut exp_agg_value = 0u64;
    for i in 0..=contributor_num {
        exp_agg_value += i as u64 + 1u64;
    }

    loop {
        match nimiq_time::timeout(
            deadline.saturating_duration_since(Instant::now()),
            aggregation.next(),
        )
        .await
        {
            Ok(Some(aggregate)) => {
                if aggregate.num_contributors() == contributor_num + 1
                    && aggregate.value == exp_agg_value
                {
                    // fully aggregated the result. break the loop here
                    break;
                }
            }
            Ok(None) => panic!("Aggregate returned a None value, which should be unreachable!()"),
            Err(_) => panic!("Aggregate took too long"),
        }
    }

    drop(aggregation);
    net.disconnect();

    // give the other aggregations time to complete themselves
    let mut finished_count = 0usize;
    loop {
        let _ = receiver.recv().await;
        finished_count += 1;
        if finished_count == contributor_num {
            break;
        }
    }

    // after we have the final aggregate create a new instance and have it (without any other instances)
    // return a fully aggregated contribution and terminate.
    // Same as before
    // let net = Arc::new(hub.new_network_with_address(contributor_num as u64));
    let protocol = Protocol::new(contributor_num, contributor_num + 1, contributor_num + 1);
    let mut contributors = BitSet::new();
    contributors.insert(contributor_num);
    let contribution = Contribution {
        value: contributor_num as u64 + 1u64,
        contributors,
    };
    for network in &networks {
        net.dial_mock(network);
    }

    // instead of spawning the aggregation task await its result here.
    let mut aggregation = Aggregation::new(
        protocol,
        config.clone(),
        contribution,
        Box::pin(
            net.receive_messages::<Update<Contribution>>()
                .map(move |msg| msg.0 .0),
        ),
        NetworkWrapper(net),
    );

    // first poll will add the nodes individual contribution and send its LevelUpdate
    // which should be responded to with a full aggregation
    let _ = aggregation.next().await;
    // Second poll must return a full contribution
    let last_aggregate = aggregation.next().await;

    // An aggregation needs to be present
    assert!(last_aggregate.is_some(), "Nothing was aggregated!");

    let last_aggregate = last_aggregate.unwrap();

    // All nodes need to contribute
    assert_eq!(
        last_aggregate.num_contributors(),
        contributor_num + 1,
        "Not all contributions are present: {:?}",
        last_aggregate,
    );

    // the final value needs to be the sum of all contributions: `exp_agg_value`
    assert_eq!(
        last_aggregate.value, exp_agg_value,
        "Wrong aggregation result",
    );

    *stopped.write() = true;
}

// additional tests:
// it_sends_periodic_updates
// it_activates_levels
