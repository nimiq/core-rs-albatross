use std::{fmt::Formatter, future::Future, sync::Arc, time::Duration};

use async_trait::async_trait;
use futures::StreamExt;
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
use tokio::sync::{
    mpsc,
    mpsc::{Receiver, Sender},
};

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
    pub fn new(node_id: usize, num_ids: usize) -> Self {
        let partitioner = Arc::new(BinomialPartitioner::new(node_id, num_ids));
        let registry = Arc::new(Registry {});
        let store = Arc::new(RwLock::new(ReplaceStore::<usize, Self>::new(
            partitioner.clone(),
        )));

        let evaluator = Arc::new(WeightedVote::new(
            store.clone(),
            Arc::clone(&registry),
            partitioner.clone(),
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
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + 'static {
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
    }
}

fn create_handel_instance(
    config: Config,
    num_contributors: usize,
    node_id: usize,
    network: Arc<MockNetwork>,
) -> Aggregation<usize, Protocol, NetworkWrapper<MockNetwork>> {
    // Create a protocol with `num_contributors` peers and set its id to `node_id`.
    let protocol = Protocol::new(node_id, num_contributors);

    // The sole contributor is this node.
    let mut contributors = BitSet::new();
    contributors.insert(node_id);

    // Create a contribution for this node with a value of `node_id + 1`.
    // This is to ensure that no node has value 0 which doesn't show up in addition.
    let contribution = Contribution {
        value: node_id as u64 + 1,
        contributors,
    };

    Aggregation::new(
        protocol,
        config.clone(),
        contribution,
        network
            .receive_messages::<Update<Contribution>>()
            .map(move |msg| msg.0 .0.into_level_update(msg.1 .0 as u16))
            .boxed(),
        NetworkWrapper(network),
    )
}

fn spawn_handel_instance(
    config: Config,
    num_contributors: usize,
    node_id: usize,
    hub: &mut MockHub,
    output: Sender<(usize, Contribution)>,
) -> Arc<MockNetwork> {
    let network = Arc::new(hub.new_network_with_address(node_id as u64));

    let mut handel =
        create_handel_instance(config, num_contributors, node_id, Arc::clone(&network));

    spawn(async move {
        loop {
            // Use a timeout here to eventually terminate the future if the aggregation stream is
            // not producing any more items.
            let item = timeout(Duration::from_secs(10), handel.next()).await;
            if let Ok(Some(contribution)) = item {
                output
                    .send((node_id, contribution))
                    .await
                    .expect("Send should not fail");
            } else {
                return;
            }
        }
    });

    network
}

async fn wait_for_contributions(
    num_contributions: usize,
    num_nodes: usize,
    receiver: &mut Receiver<(usize, Contribution)>,
) {
    let mut count = 0;
    while count < num_nodes {
        let (_, contribution) = timeout(Duration::from_secs(3), receiver.recv())
            .await
            .ok()
            .flatten()
            .expect("Aggregation took too long");
        if contribution.num_contributors() == num_contributions {
            count += 1;
        }
    }
}

#[test(tokio::test)]
async fn handel_happy_path() {
    let num_contributors = 20;
    let config = Config {
        update_interval: Duration::from_millis(100),
        level_timeout: Duration::from_millis(500),
        peer_count: 1,
    };

    let mut hub = MockHub::default();
    let mut networks: Vec<Arc<MockNetwork>> = vec![];
    let (tx, mut rx) = mpsc::channel(1024);

    // Spawn `num_contributors` handel instances and connect them to each other.
    for node_id in 0..num_contributors {
        let network = spawn_handel_instance(
            config.clone(),
            num_contributors,
            node_id,
            &mut hub,
            tx.clone(),
        );

        // Connect the network to all already existing ones.
        for net in &networks {
            network.dial_mock(net);
        }
        networks.push(network);
    }

    // Wait for all aggregations to complete.
    wait_for_contributions(num_contributors, num_contributors, &mut rx).await;
}

#[test(tokio::test)]
async fn handel_one_node_late() {
    let num_contributors = 17;
    let config = Config {
        update_interval: Duration::from_millis(100),
        level_timeout: Duration::from_millis(500),
        peer_count: 1,
    };

    let mut hub = MockHub::default();
    let mut networks: Vec<Arc<MockNetwork>> = vec![];
    let (tx, mut rx) = mpsc::channel(1024);

    // Spawn `num_contributors - 1` handel instances and connect them to each other.
    for node_id in 0..num_contributors - 1 {
        let network = spawn_handel_instance(
            config.clone(),
            num_contributors,
            node_id,
            &mut hub,
            tx.clone(),
        );

        // Connect the network to all already existing ones.
        for net in &networks {
            network.dial_mock(net);
        }
        networks.push(network);
    }

    // Wait for all aggregations to collect `num_contributors - 1` contributions.
    wait_for_contributions(num_contributors - 1, num_contributors - 1, &mut rx).await;

    // Spawn the late node.
    let network = spawn_handel_instance(
        config.clone(),
        num_contributors,
        num_contributors - 1,
        &mut hub,
        tx.clone(),
    );

    // Connect the network to all already existing ones.
    for net in &networks {
        network.dial_mock(net);
    }

    // Wait for all aggregations to complete.
    wait_for_contributions(num_contributors, num_contributors, &mut rx).await;
}

#[test(tokio::test)]
#[ignore]
async fn handel_poor_connectivity() {
    let num_contributors = 4;
    let config = Config {
        update_interval: Duration::from_millis(100),
        level_timeout: Duration::from_millis(500),
        peer_count: 1,
    };

    let mut hub = MockHub::default();
    let mut prev_net: Option<Arc<MockNetwork>> = None;
    let (tx, mut rx) = mpsc::channel(1024);

    // Spawn `num_contributors` handel instances.
    for node_id in 0..num_contributors {
        let network = spawn_handel_instance(
            config.clone(),
            num_contributors,
            node_id,
            &mut hub,
            tx.clone(),
        );

        // Connect each node only to the previous one.
        if let Some(net) = prev_net {
            network.dial_mock(&net);
        }
        prev_net = Some(network);
    }

    // Wait for all aggregations to complete.
    wait_for_contributions(num_contributors, num_contributors, &mut rx).await;
}

#[test(tokio::test)]
async fn handel_netsplit() {
    let num_contributors = 18;
    let config = Config {
        update_interval: Duration::from_millis(100),
        level_timeout: Duration::from_millis(300),
        peer_count: 1,
    };

    let mut hub = MockHub::default();
    let mut networks1: Vec<Arc<MockNetwork>> = vec![];
    let mut networks2: Vec<Arc<MockNetwork>> = vec![];
    let (tx, mut rx) = mpsc::channel(1024);

    // Spawn `num_contributors` handel instances.
    for node_id in 0..num_contributors {
        let network = spawn_handel_instance(
            config.clone(),
            num_contributors,
            node_id,
            &mut hub,
            tx.clone(),
        );

        // Split handel instances into two clusters.
        let networks = if node_id % 2 == 0 {
            &mut networks1
        } else {
            &mut networks2
        };
        for net in networks.iter() {
            network.dial_mock(net);
        }
        networks.push(network);
    }

    // Wait for all aggregations to collect half of the contributions.
    wait_for_contributions(num_contributors / 2, num_contributors, &mut rx).await;

    // Now connect the two clusters.
    for net1 in &networks1 {
        for net2 in &networks2 {
            net1.dial_mock(net2);
        }
    }

    // Wait for all aggregations to complete.
    wait_for_contributions(num_contributors, num_contributors, &mut rx).await;
}
