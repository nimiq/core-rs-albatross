use std::fmt;
use std::pin::Pin;
use std::sync::Arc;

use futures::channel::mpsc::{unbounded, UnboundedReceiver, UnboundedSender};
use futures::ready;
use futures::stream::{BoxStream, Stream, StreamExt};
use futures::task::{Context, Poll};
use parking_lot::RwLock;

use beserial::{Deserialize, Serialize};
use block::{Message, MultiSignature, SignedViewChange, ViewChange, ViewChangeProof};
use bls::AggregatePublicKey;
use collections::BitSet;
use handel::aggregation::Aggregation;
use handel::config::Config;
use handel::contribution::{AggregatableContribution, ContributionError};
use handel::evaluator::WeightedVote;
use handel::identity::WeightRegistry;
use handel::partitioner::BinomialPartitioner;
use handel::protocol::Protocol;
use handel::store::ReplaceStore;
use handel::update::{LevelUpdate, LevelUpdateMessage};
use hash::Blake2sHash;
use nimiq_validator_network::ValidatorNetwork;
use primitives::policy;
use primitives::slots::Validators;

use super::network_sink::NetworkSink;
use super::registry::ValidatorRegistry;
use super::verifier::MultithreadedVerifier;

enum ViewChangeResult {
    FutureViewChange(SignedViewChangeMessage, ViewChange),
    ViewChange(SignedViewChangeMessage),
}

/// Switch for incoming ViewChanges.
/// Keeps track of viewChanges for future Aggregations in order to be able to sync the state of this node with others
/// in case it recognizes it is behind.
struct InputStreamSwitch {
    input: BoxStream<'static, LevelUpdateMessage<SignedViewChangeMessage, ViewChange>>,
    sender: UnboundedSender<ViewChangeResult>,
    future_view_changes: BitSet,
    current_view_change: ViewChange,
    identity_registry: Arc<ValidatorRegistry>,
}

impl InputStreamSwitch {
    fn new(
        input: BoxStream<'static, LevelUpdateMessage<SignedViewChangeMessage, ViewChange>>,
        current_view_change: ViewChange,
        identity_registry: Arc<ValidatorRegistry>,
    ) -> (Self, UnboundedReceiver<ViewChangeResult>) {
        let (sender, receiver) = unbounded::<ViewChangeResult>();

        let this = Self {
            input,
            sender,
            future_view_changes: BitSet::new(),
            current_view_change,
            identity_registry,
        };

        (this, receiver)
    }
}

impl Stream for InputStreamSwitch {
    type Item = LevelUpdate<SignedViewChangeMessage>;
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        while let Some(message) = ready!(self.input.poll_next_unpin(cx)) {
            if message.tag.block_number != self.current_view_change.block_number
                || message.tag.vrf_entropy != self.current_view_change.vrf_entropy
            {
                // The LevelUpdate is not for this view change and thus irrelevant.
                // TODO If it is for a future view change we might want to shortcut a HeadRequest here.
                continue;
            }

            if message.tag.new_view_number == self.current_view_change.new_view_number {
                return Poll::Ready(Some(message.update));
            }

            if message.tag.new_view_number > self.current_view_change.new_view_number {
                let result =
                    ViewChangeResult::FutureViewChange(message.update.aggregate, message.tag);
                if let Err(err) = self.sender.unbounded_send(result) {
                    error!("Failed to send FutureViewChange result: {:?}", err);
                }
            }
        }

        // We have exited the loop, so poll_next() must have returned Poll::Ready(None).
        // Thus, we terminate the stream.
        Poll::Ready(None)
    }
}

// TODO once actual state sync is implemented this can be removed again as it serves the same purpose.
/// The ViewChangeMessage containing the current view change and an optional previous proof if applicable.
/// Contains the actual information of block_height, new_view_number and prev_seed as tag.
#[derive(Clone, Deserialize, Serialize, std::fmt::Debug)]
pub struct SignedViewChangeMessage {
    /// The currently aggregated view change.
    pub view_change: MultiSignature,
    /// The view ChangeProof of the previous view change on this height, if there is one.
    /// The view_change parameters (block_number, new_view_number, prev_seed are implicit).
    pub previous_proof: Option<MultiSignature>,
}

impl AggregatableContribution for SignedViewChangeMessage {
    fn contributors(&self) -> BitSet {
        self.view_change.contributors()
    }

    fn combine(&mut self, other_contribution: &Self) -> Result<(), ContributionError> {
        self.view_change.combine(&other_contribution.view_change)
    }
}

struct ViewChangeAggregationProtocol {
    verifier: Arc<<Self as Protocol>::Verifier>,
    partitioner: Arc<<Self as Protocol>::Partitioner>,
    evaluator: Arc<<Self as Protocol>::Evaluator>,
    store: Arc<RwLock<<Self as Protocol>::Store>>,
    registry: Arc<<Self as Protocol>::Registry>,

    node_id: usize,
}

impl ViewChangeAggregationProtocol {
    pub fn new(
        validators: Validators,
        node_id: usize,
        threshold: usize,
        message_hash: Blake2sHash,
    ) -> Self {
        let partitioner = Arc::new(BinomialPartitioner::new(
            node_id,
            validators.num_validators(),
        ));

        let store = Arc::new(RwLock::new(ReplaceStore::<
            BinomialPartitioner,
            SignedViewChangeMessage,
        >::new(Arc::clone(&partitioner))));

        let registry = Arc::new(ValidatorRegistry::new(validators));

        let evaluator = Arc::new(WeightedVote::new(
            Arc::clone(&store),
            Arc::clone(&registry),
            Arc::clone(&partitioner),
            threshold,
        ));

        ViewChangeAggregationProtocol {
            verifier: Arc::new(MultithreadedVerifier::new(
                message_hash,
                Arc::clone(&registry),
            )),
            partitioner,
            evaluator,
            store,
            registry,
            node_id,
        }
    }
}

impl Protocol for ViewChangeAggregationProtocol {
    type Contribution = SignedViewChangeMessage;
    type Registry = ValidatorRegistry;
    type Verifier = MultithreadedVerifier<Self::Registry>;
    type Store = ReplaceStore<Self::Partitioner, Self::Contribution>;
    type Evaluator = WeightedVote<Self::Store, Self::Registry, Self::Partitioner>;
    type Partitioner = BinomialPartitioner;

    fn registry(&self) -> Arc<Self::Registry> {
        self.registry.clone()
    }

    fn verifier(&self) -> Arc<Self::Verifier> {
        self.verifier.clone()
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

pub struct ViewChangeAggregation {}

impl ViewChangeAggregation {
    pub async fn start<N: ValidatorNetwork + 'static>(
        mut view_change: ViewChange,
        mut previous_proof: Option<MultiSignature>,
        voting_key: bls::KeyPair,
        // TODO: This seems to be a SlotBand. Change this to a proper Validator ID.
        validator_id: u16,
        active_validators: Validators,
        network: Arc<N>,
    ) -> (ViewChange, ViewChangeProof) {
        // TODO expose this somewehere else so we don't need to clone here.
        let weights = Arc::new(ValidatorRegistry::new(active_validators.clone()));

        let slot_range = active_validators.validators[validator_id as usize].slot_range;

        let slots: Vec<u16> = (slot_range.0..slot_range.1).collect();

        trace!("Previous view_change proof: {:?}", &previous_proof);

        loop {
            let message_hash = view_change.hash_with_prefix();
            trace!(
                "message: {:?}, message_hash: {:?}",
                &view_change,
                message_hash
            );
            let signed_view_change = SignedViewChange::from_message(
                view_change.clone(),
                &voting_key.secret_key,
                validator_id,
            );

            let signature = bls::AggregateSignature::from_signatures(&[signed_view_change
                .signature
                .multiply(slots.len() as u16)]);

            let mut signers = BitSet::new();
            for slot in &slots {
                signers.insert(*slot as usize);
            }

            let own_contribution = SignedViewChangeMessage {
                view_change: MultiSignature::new(signature, signers),
                previous_proof: previous_proof.clone(),
            };

            warn!(
                "Starting view change {}.{}",
                &view_change.block_number, &view_change.new_view_number,
            );

            let protocol = ViewChangeAggregationProtocol::new(
                active_validators.clone(),
                validator_id as usize,
                policy::TWO_F_PLUS_ONE as usize,
                message_hash,
            );

            let (input_switch, receiver) = InputStreamSwitch::new(
                Box::pin(
                    network
                        .receive::<LevelUpdateMessage<SignedViewChangeMessage, ViewChange>>()
                        .map(move |msg| msg.0),
                ),
                view_change.clone(),
                weights.clone(),
            );

            let aggregation = Aggregation::new(
                protocol,
                view_change.clone(),
                Config::default(),
                own_contribution,
                Box::pin(input_switch),
                Box::new(NetworkSink::<
                    LevelUpdateMessage<SignedViewChangeMessage, ViewChange>,
                    N,
                >::new(network.clone())),
            );

            let mut stream =
                futures::stream::select(aggregation.map(ViewChangeResult::ViewChange), receiver);
            while let Some(msg) = stream.next().await {
                match msg {
                    ViewChangeResult::FutureViewChange(vc, tag) => {
                        debug!("Received future ViewChange: {:?}", &vc);
                        if let Some(sig) = vc.previous_proof {
                            // verify the proof
                            // fist aggregate the public keys
                            let mut aggregated_public_key = AggregatePublicKey::new();
                            for signer in sig.signers.iter() {
                                aggregated_public_key.aggregate(
                                    &active_validators
                                        .get_validator_by_slot_number(signer as u16)
                                        .voting_key
                                        .uncompress()
                                        .expect("Could not uncompress lazyPublicKey"),
                                );
                            }

                            let past_view_change = ViewChange {
                                block_number: tag.block_number,
                                new_view_number: tag.new_view_number - 1,
                                vrf_entropy: tag.vrf_entropy.clone(),
                            };

                            // verify the ViewChange
                            if aggregated_public_key
                                .verify_hash(past_view_change.hash_with_prefix(), &sig.signature)
                            {
                                // set the proof and exit the while loop to create a new Aggregtion for the correct new view
                                view_change = tag;
                                previous_proof = Some(sig);
                                break;
                            }
                        }
                        error!("Did not receive necessary past proof!");
                    }
                    ViewChangeResult::ViewChange(vc) => {
                        if let Some(aggregate_weight) = weights.signature_weight(&vc.view_change) {
                            trace!(
                                "New View Change Aggregate weight: {} / {} Signers: {:?}",
                                aggregate_weight,
                                policy::TWO_F_PLUS_ONE,
                                &vc.view_change.contributors(),
                            );

                            // Check if the combined weight of the aggregation is at least 2f+1.
                            if aggregate_weight >= policy::TWO_F_PLUS_ONE as usize {
                                // Create ViewChangeProof out of the aggregate
                                let view_change_proof = ViewChangeProof {
                                    sig: vc.view_change,
                                };
                                trace!("View Change complete: {:?}", &view_change_proof);

                                // return the ViewChangeProof
                                return (view_change, view_change_proof);
                            }
                        }
                    }
                }
            }
        }
    }
}

impl fmt::Debug for ViewChangeAggregationProtocol {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "ViewChangeAggregation {{ node_id: {} }}", self.node_id(),)
    }
}
