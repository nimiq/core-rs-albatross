#[macro_use]
extern crate beserial_derive;

use async_trait::async_trait;
use beserial::{Deserialize, Serialize};
use identity::Identity;
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

use futures::future::BoxFuture;
use futures::sink::Sink;
use futures::stream::StreamExt;
use futures::task::{Context, Poll};
use std::marker::PhantomData;
use std::pin::Pin;
use std::sync::Arc;
use std::{fmt::Formatter, time::Duration};

use parking_lot::RwLock;



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

    fn signers_identity(&self, signers: &BitSet) -> Identity {
        if signers.len() == 1 {
            Identity::Single(signers.iter().next().unwrap())
        } else {
            Identity::None
        }
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

struct SendingFuture<N: Network> {
    network: Arc<N>,
}

impl<N: Network> SendingFuture<N> {
    pub async fn send<M: Message + Unpin + std::fmt::Debug>(self, msg: M) {
        self.network.broadcast(&msg).await
    }
}

/// Implementation of a simple Sink Wrapper for the NetworkInterface's Network trait
pub struct NetworkSink<M: Message + Unpin, N: Network> {
    /// The network this sink is sending its messages over
    network: Arc<N>,
    /// The currently executed future of sending an item.
    current_future: Option<BoxFuture<'static, ()>>,

    phantom: PhantomData<M>,
}

impl<M: Message + Unpin, N: Network> NetworkSink<M, N> {
    pub fn new(network: Arc<N>) -> Self {
        Self {
            network,
            current_future: None,
            phantom: PhantomData,
        }
    }
}

impl<M: Message + Unpin + std::fmt::Debug, N: Network> Sink<(M, usize)> for NetworkSink<M, N> {
    type Error = ();

    fn poll_ready(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        // As this Sink only bufferes a single message poll_ready is the same as poll_flush
        self.poll_flush(cx)
    }

    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        // As this Sink only bufferes a single message poll_close is the same as poll_flush
        self.poll_flush(cx)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        // If there is a future being processed
        if let Some(mut fut) = self.current_future.take() {
            // Poll it to check its state
            if fut.as_mut().poll(cx).is_pending() {
                // If it is still being processed reset self.current_future and return Pending as no new item can be accepted (and the buffer is occupied).
                self.current_future = Some(fut);
                Poll::Pending
            } else {
                // If it has completed a new item can be accepted (and the buffer is also empty).
                Poll::Ready(Ok(()))
            }
        } else {
            // when there is no future the buffer is empty and a new item can be accepted.
            Poll::Ready(Ok(()))
        }
    }

    fn start_send(mut self: Pin<&mut Self>, item: (M, usize)) -> Result<(), Self::Error> {
        // If there is future poll_ready didnot return Ready(Ok(())) or poll_ready was not called resulting in an error
        if self.current_future.is_some() {
            Err(())
        } else {
            // Otherwise, create the future and store it.
            // Note: This future does not get polled. Only once poll_* is called it will actually be polled.
            let fut = Box::pin(SendingFuture { network: self.network.clone() }.send(item.0)); //.boxed();
            self.current_future = Some(fut);
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
        let net = Arc::new(hub.new_network_with_address(id as u64));
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
            1_u8, // serves as the tag or identifier for this aggregation
            config.clone(),
            contribution,
            Box::pin(net.receive_from_all::<LevelUpdateMessage<Contribution, u8>>().map(move |msg| msg.0.update)),
            Box::new(NetworkSink {
                network: net.clone(),
                current_future: None,
                phantom: PhantomData,
            }),
        );

        tokio::spawn(async move {
            // have them just run until the aggregation is finished
            while let Some(_contribution) = aggregation.next().await {}
            println!("{} is done", &id);
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
    networks.push(net.clone());

    // instead of spawning the aggregation task await its result here.
    let mut aggregation = Aggregation::new(
        protocol,
        1_u8, // serves as the tag or identifier for this aggregation
        config.clone(),
        contribution,
        Box::pin(net.receive_from_all::<LevelUpdateMessage<Contribution, u8>>().map(move |msg| msg.0.update)),
        Box::new(NetworkSink {
            network: net.clone(),
            current_future: None,
            phantom: PhantomData,
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
    assert_eq!(last_aggregate.num_contributors(), contributor_num + 1, "Not all contributions are present",);

    // the final value needs to be the sum of all contributions: 8 + 7 + 6 + 5 + 4 + 3 + 2 + 1 = 36
    assert_eq!(last_aggregate.value, 36, "Wrong aggregation result",);
}

// additional tests:
// it_sends_periodic_updates
// it_activates_levels
