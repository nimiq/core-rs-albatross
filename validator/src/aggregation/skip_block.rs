use std::{
    fmt,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    time::Duration,
};

use futures::{
    future::FutureExt,
    ready,
    stream::{BoxStream, Stream, StreamExt},
};
use nimiq_block::{MultiSignature, SignedSkipBlockInfo, SkipBlockInfo, SkipBlockProof};
use nimiq_bls::{AggregateSignature, KeyPair};
use nimiq_collections::BitSet;
use nimiq_handel::{
    aggregation::Aggregation,
    config::Config,
    contribution::{AggregatableContribution, ContributionError},
    evaluator::WeightedVote,
    identity::WeightRegistry,
    partitioner::BinomialPartitioner,
    protocol::Protocol,
    store::ReplaceStore,
    update::LevelUpdate,
};
use nimiq_hash::Blake2sHash;
use nimiq_network_interface::request::{MessageMarker, RequestCommon};
use nimiq_primitives::{policy, slots_allocation::Validators, Message};
use nimiq_validator_network::ValidatorNetwork;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};

use super::{registry::ValidatorRegistry, verifier::MultithreadedVerifier};

enum SkipBlockResult {
    SkipBlock(SignedSkipBlockMessage),
}

/// Switch for incoming SkipBlockInfo.
/// Keeps track of SkipBlockInfo for future Aggregations in order to be able to sync the state of this node with others
/// in case it recognizes it is behind.
struct InputStreamSwitch {
    input: BoxStream<'static, SkipBlockUpdate>,
    current_skip_block: SkipBlockInfo,
}

impl InputStreamSwitch {
    fn new(input: BoxStream<'static, SkipBlockUpdate>, current_skip_block: SkipBlockInfo) -> Self {
        Self {
            input,
            current_skip_block,
        }
    }
}

impl Stream for InputStreamSwitch {
    type Item = LevelUpdate<SignedSkipBlockMessage>;
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        while let Some(message) = ready!(self.input.poll_next_unpin(cx)) {
            if message.info.block_number != self.current_skip_block.block_number
                || message.info.vrf_entropy != self.current_skip_block.vrf_entropy
            {
                // The LevelUpdate is not for this skip block and thus irrelevant.
                // TODO If it is for a future skip block we might want to shortcut a HeadRequest here.
                continue;
            }

            return Poll::Ready(Some(message.level_update));
        }

        // We have exited the loop, so poll_next() must have returned Poll::Ready(None).
        // Thus, we terminate the stream.
        Poll::Ready(None)
    }
}

struct NetworkWrapper<TValidatorNetwork: ValidatorNetwork> {
    network: Arc<TValidatorNetwork>,
    tag: SkipBlockInfo,
}

impl<TValidatorNetwork: ValidatorNetwork> NetworkWrapper<TValidatorNetwork> {
    fn new(tag: SkipBlockInfo, network: Arc<TValidatorNetwork>) -> Self {
        Self { network, tag }
    }
}
impl<TValidatorNetwork: ValidatorNetwork + 'static> nimiq_handel::network::Network
    for NetworkWrapper<TValidatorNetwork>
{
    type Contribution = SignedSkipBlockMessage;

    fn send_to(
        &self,
        (msg, recipient): (LevelUpdate<Self::Contribution>, u16),
    ) -> futures::future::BoxFuture<'static, ()> {
        // Create the update.
        let update_message = SkipBlockUpdate {
            level_update: msg,
            info: self.tag.clone(),
        };

        // clone network so it can be moved into the future
        let nw = Arc::clone(&self.network);

        // create the send future and return it.
        async move {
            if let Err(error) = nw.send_to(recipient, update_message).await {
                log::error!(?error, recipient, "Failed to send message");
            }
        }
        .boxed()
    }
}

/// The SignedSkipBlockMessage containing the current skip block proof.
/// Contains the actual information of block_height and prev_seed as tag (SkipBlockInfo).
#[derive(Clone, Deserialize, Serialize, std::fmt::Debug)]
pub struct SignedSkipBlockMessage {
    /// The currently aggregated proof for a skip block.
    pub proof: MultiSignature,
}

impl AggregatableContribution for SignedSkipBlockMessage {
    fn contributors(&self) -> BitSet {
        self.proof.contributors()
    }

    fn combine(&mut self, other_contribution: &Self) -> Result<(), ContributionError> {
        self.proof
            .combine(&other_contribution.proof)
            .map_err(ContributionError::Overlapping)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct SkipBlockUpdate {
    pub level_update: LevelUpdate<SignedSkipBlockMessage>,
    pub info: SkipBlockInfo,
}

impl RequestCommon for SkipBlockUpdate {
    type Kind = MessageMarker;
    const TYPE_ID: u16 = 123;
    const MAX_REQUESTS: u32 = 500;
    const TIME_WINDOW: Duration = Duration::from_millis(500);
    type Response = ();
}

struct SkipBlockAggregationProtocol {
    verifier: Arc<<Self as Protocol<u32>>::Verifier>,
    partitioner: Arc<<Self as Protocol<u32>>::Partitioner>,
    evaluator: Arc<<Self as Protocol<u32>>::Evaluator>,
    store: Arc<RwLock<<Self as Protocol<u32>>::Store>>,
    registry: Arc<<Self as Protocol<u32>>::Registry>,

    node_id: usize,
    block_height: u32,
}

impl SkipBlockAggregationProtocol {
    pub fn new(
        validators: Validators,
        node_id: usize,
        threshold: usize,
        message_hash: Blake2sHash,
        block_height: u32,
    ) -> Self {
        let partitioner = Arc::new(BinomialPartitioner::new(
            node_id,
            validators.num_validators(),
        ));

        let store = Arc::new(RwLock::new(ReplaceStore::<u32, Self>::new(Arc::clone(
            &partitioner,
        ))));

        let registry = Arc::new(ValidatorRegistry::new(validators));

        let evaluator = Arc::new(WeightedVote::new(
            Arc::clone(&store),
            Arc::clone(&registry),
            Arc::clone(&partitioner),
            threshold,
        ));

        SkipBlockAggregationProtocol {
            verifier: Arc::new(MultithreadedVerifier::new(
                message_hash,
                Arc::clone(&registry),
            )),
            partitioner,
            evaluator,
            store,
            registry,
            node_id,
            block_height,
        }
    }
}

impl Protocol<u32> for SkipBlockAggregationProtocol {
    type Contribution = SignedSkipBlockMessage;
    type Registry = ValidatorRegistry;
    type Verifier = MultithreadedVerifier<Self::Registry>;
    type Store = ReplaceStore<u32, Self>;
    type Evaluator = WeightedVote<u32, Self>;
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

    fn identify(&self) -> u32 {
        self.block_height
    }

    fn node_id(&self) -> usize {
        self.node_id
    }
}

pub struct SkipBlockAggregation {}

impl SkipBlockAggregation {
    pub async fn start<N: ValidatorNetwork + 'static>(
        skip_block_info: SkipBlockInfo,
        voting_key: KeyPair,
        // TODO: This seems to be a SlotBand. Change this to a proper Validator ID.
        validator_id: u16,
        active_validators: Validators,
        network: Arc<N>,
    ) -> (SkipBlockInfo, SkipBlockProof) {
        // TODO expose this somewehere else so we don't need to clone here.
        let weights = Arc::new(ValidatorRegistry::new(active_validators.clone()));

        let slots = active_validators.validators[validator_id as usize]
            .slots
            .clone();

        let message_hash = skip_block_info.hash_with_prefix();
        trace!(
            %message_hash,
            ?skip_block_info,
            "Starting skip block aggregation",
        );
        let signed_skip_block_info = SignedSkipBlockInfo::from_message(
            skip_block_info.clone(),
            &voting_key.secret_key,
            validator_id,
        );

        let signature = AggregateSignature::from_signatures(&[signed_skip_block_info
            .signature
            .multiply(slots.len() as u16)]);

        let mut signers = BitSet::new();
        for slot in slots.clone() {
            signers.insert(slot as usize);
        }

        let own_contribution = SignedSkipBlockMessage {
            proof: MultiSignature::new(signature, signers),
        };

        let protocol = SkipBlockAggregationProtocol::new(
            active_validators.clone(),
            validator_id as usize,
            policy::Policy::TWO_F_PLUS_ONE as usize,
            message_hash,
            skip_block_info.block_number,
        );

        let input_switch = InputStreamSwitch::new(
            Box::pin(
                network
                    .receive::<SkipBlockUpdate>()
                    .filter_map(|(update, sender_id)| {
                        futures::future::ready(
                            (sender_id == update.level_update.origin()).then(|| update),
                        )
                    }),
            ),
            skip_block_info.clone(),
        );

        let aggregation = Aggregation::new(
            protocol,
            Config::default(),
            own_contribution,
            Box::pin(input_switch),
            NetworkWrapper::new(skip_block_info.clone(), Arc::clone(&network)),
        );

        let mut stream = aggregation.map(SkipBlockResult::SkipBlock);
        while let Some(msg) = stream.next().await {
            match msg {
                SkipBlockResult::SkipBlock(sb_msg) => {
                    if let Some(aggregate_weight) = weights.signature_weight(&sb_msg) {
                        info!(
                            aggregate_weight,
                            signers = %sb_msg.contributors(),
                            "New skip block aggregate weight {}/{} with signers {}",
                            aggregate_weight,
                            policy::Policy::TWO_F_PLUS_ONE,
                            &sb_msg.contributors(),
                        );

                        // Check if the combined weight of the aggregation is at least 2f+1.
                        if aggregate_weight >= policy::Policy::TWO_F_PLUS_ONE as usize {
                            // Create SkipBlockProof out of the aggregate
                            let skip_block_proof = SkipBlockProof { sig: sb_msg.proof };
                            trace!("Skip block completed, proof={:?}", &skip_block_proof);

                            // return the SkipBlockProof
                            return (skip_block_info, skip_block_proof);
                        }
                    }
                }
            }
        }

        unreachable!("Aggregation stream should not terminate without result");
    }
}

impl fmt::Debug for SkipBlockAggregationProtocol {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "SkipBlockAggregation {{ node_id: {} }}", self.node_id(),)
    }
}
