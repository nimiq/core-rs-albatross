#[macro_use]
extern crate beserial_derive;

use async_trait::async_trait;
use beserial::{Deserialize, Serialize};
use nimiq_bls::PublicKey;
use nimiq_collections::bitset::BitSet;
use nimiq_handel::aggregation::Aggregation;
use nimiq_handel::config::Config;
use nimiq_handel::contribution::{AggregatableContribution, ContributionError};
use nimiq_handel::evaluator;
use nimiq_handel::identity;
use nimiq_handel::partitioner::BinomialPartitioner;
use nimiq_handel::protocol;
use nimiq_handel::store::ReplaceStore;
use nimiq_handel::update::LevelUpdateMessage;
use nimiq_handel::verifier;
use nimiq_network_interface::message::Message;
use nimiq_network_interface::network::Network;
use nimiq_network_mock::{MockHub, MockNetwork};

use futures::future::Future;
use futures::sink::Sink;
use futures::stream::StreamExt;
use futures::task::{Context, Poll};
use std::pin::Pin;
use std::sync::Arc;
use std::{fmt::Formatter, time::Duration};

use parking_lot::RwLock;

use tokio;

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
impl identity::WeightRegistry for Registry {
    fn weight(&self, _id: usize) -> Option<usize> {
        Some(1)
    }
}
impl identity::IdentityRegistry for Registry {
    fn public_key(&self, _id: usize) -> Option<PublicKey> {
        None
    }
}

/// A dump Verifier who is happy with everything.
pub struct DumbVerifier {}

#[async_trait]
impl verifier::Verifier for DumbVerifier {
    type Contribution = Contribution;
    async fn verify(&self, _contribution: &Self::Contribution) -> verifier::VerificationResult {
        verifier::VerificationResult::Ok
    }
}

pub type Store = ReplaceStore<BinomialPartitioner, Contribution>;

pub type Evaluator = evaluator::WeightedVote<Store, Registry, BinomialPartitioner>;

// The test protocol combining the other types.
pub struct Protocol {
    verifier: Arc<DumbVerifier>,
    partitioner: Arc<BinomialPartitioner>,
    evaluator: Arc<Evaluator>,
    store: Arc<RwLock<ReplaceStore<BinomialPartitioner, Contribution>>>,
    registry: Arc<Registry>,
    node_id: usize,
}
impl Protocol {
    pub fn new(node_id: usize, num_ids: usize, threshold: usize) -> Self {
        let partitioner = Arc::new(BinomialPartitioner::new(node_id, num_ids));
        let registry = Arc::new(Registry {});
        let store = Arc::new(RwLock::new(ReplaceStore::<BinomialPartitioner, Contribution>::new(partitioner.clone())));

        let evaluator = Arc::new(evaluator::WeightedVote::new(
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

impl protocol::Protocol for Protocol {
    type Contribution = Contribution;
    type Verifier = DumbVerifier;
    type Registry = Registry;
    type Partitioner = BinomialPartitioner;
    type Store = Store;
    type Evaluator = evaluator::WeightedVote<Self::Store, Self::Registry, Self::Partitioner>;

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
    fn node_id(&self) -> usize {
        self.node_id
    }
}

/// Dump Sink wrapper for Network trait. (Only to be used with MockNetwork or the like, which have instant sending).
struct NetworkSink<M: Message, N: Network> {
    /// The network implementation this Sink is sending messages over
    network: Arc<N>,
    /// The buffered message if there is one
    buffered_message: Option<M>,
}

impl<N: Network> Sink<(LevelUpdateMessage<Contribution, u8>, usize)>
    for NetworkSink<LevelUpdateMessage<Contribution, u8>, N>
{
    type Error = ();

    fn poll_ready(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        if self.buffered_message.is_some() {
            Poll::Pending
        } else {
            Poll::Ready(Ok(()))
        }
    }

    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.poll_flush(cx)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        let msg = self.buffered_message.take().unwrap();
        let mut future = Box::pin(self.network.broadcast(&msg));
        match future.as_mut().poll(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(_) => Poll::Ready(Ok(())),
        }
    }

    fn start_send(
        mut self: Pin<&mut Self>,
        item: (LevelUpdateMessage<Contribution, u8>, usize),
    ) -> Result<(), Self::Error> {
        // TODO send_to. `item.1` is recipient.
        if self.buffered_message.is_some() {
            Err(())
        } else {
            self.buffered_message = Some(item.0);
            Ok(())
        }
    }
}

#[tokio::test]
async fn it_can_aggregate() {
    let config = Config {
        update_count: 4,
        update_interval: Duration::from_millis(500),
        timeout: Duration::from_millis(500),
        peer_count: 1,
    };

    let mut hub = MockHub::default();

    let contributor_num: usize = 7;

    let mut networks: Vec<Arc<MockNetwork>> = vec![];
    // Initialize `contributor_num networks and Handel Aggregations. Connect all the networks with each other.
    for id in 0..contributor_num {
        // Create a network with id = `id`
        let net = Arc::new(hub.new_network_with_address(id));
        // Create a protocol with `contributor_num + 1` peers set its id to `id`. Require `contributor_num` contributions
        // meaning all contributions need to be aggregated with the additional node initialized after this for loop.
        let protocol = Protocol::new(id, contributor_num + 1, contributor_num);
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
            1 as u8, // serves as the tag or identifier for this aggregation
            config.clone(),
            contribution,
            Box::pin(
                net.receive_from_all::<LevelUpdateMessage<Contribution, u8>>()
                    .map(move |msg| msg.0.update),
            ),
            Box::new(NetworkSink {
                network: net.clone(),
                buffered_message: None,
            }),
        );

        tokio::spawn(async move {
            // have them just run until the aggregation is finished
            while let Some(_) = aggregation.next().await {}
        });
    }

    // same as in the for loop, execpt we want to keep the handel sinatnce and not spawn it.
    let net = Arc::new(hub.new_network_with_address(contributor_num));
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
    networks.push(net.clone());

    // instead of spawning the aggregation atsk await its result here.
    let mut aggregation = Aggregation::new(
        protocol,
        1 as u8, // serves as the tag or identifier for this aggregation
        config.clone(),
        contribution,
        Box::pin(
            net.receive_from_all::<LevelUpdateMessage<Contribution, u8>>()
                .map(move |msg| msg.0.update),
        ),
        Box::new(NetworkSink {
            network: net.clone(),
            buffered_message: None,
        }),
    );

    let mut last_aggregate: Option<Contribution> = None;

    while let Some(aggregate) = aggregation.next().await {
        last_aggregate = Some(aggregate);
    }

    // An aggregation needs to be present
    assert!(last_aggregate.is_some(), "Nothing was aggregated!");

    let last_aggregate = last_aggregate.unwrap();

    // All nodes need to contribute
    assert_eq!(
        last_aggregate.num_contributors(),
        contributor_num + 1,
        "Not all contributions are present",
    );

    // the final value needs to be the sum of all contributions: 8 + 7 + 6 + 5 + 4 + 3 + 2 + 1 = 36
    assert_eq!(last_aggregate.value, 36, "Wrong aggregation result",);
}

// additional tests:
// it_sends_periodic_updates
// it_activates_levels
