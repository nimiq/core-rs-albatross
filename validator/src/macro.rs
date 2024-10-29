use std::{
    marker::PhantomData,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use futures::stream::{BoxStream, Stream, StreamExt};
use nimiq_block::{DoubleProposalProof, MacroBlock, MacroHeader};
use nimiq_blockchain::{BlockProducer, Blockchain};
use nimiq_keys::{Address, Ed25519Signature as SchnorrSignature};
use nimiq_network_interface::network::Topic;
use nimiq_primitives::{networks::NetworkId, slots_allocation::Validators, TendermintProposal};
use nimiq_tendermint::{
    Return as TendermintReturn, SignedProposalMessage, TaggedAggregationMessage, Tendermint,
};
use nimiq_validator_network::{PubsubId, ValidatorNetwork};
use parking_lot::{Mutex, RwLock};

use crate::{
    aggregation::tendermint::{
        contribution::AggregateMessage,
        proposal::{Header, SignedProposal},
        state::MacroState,
        update_message::TendermintUpdate,
    },
    double_proposal::DoubleProposalDetector,
    tendermint::TendermintProtocol,
};

pub(crate) enum MappedReturn<TValidatorNetwork: ValidatorNetwork + 'static>
where
    PubsubId<TValidatorNetwork>: std::fmt::Debug + Unpin,
{
    Update(MacroState),
    Decision(MacroBlock),
    ProposalAccepted(
        SignedProposalMessage<
            Header<PubsubId<TValidatorNetwork>>,
            (SchnorrSignature, Address, u16),
        >,
    ),
    ProposalIgnored(
        SignedProposalMessage<
            Header<PubsubId<TValidatorNetwork>>,
            (SchnorrSignature, Address, u16),
        >,
    ),
    ProposalRejected(
        SignedProposalMessage<
            Header<PubsubId<TValidatorNetwork>>,
            (SchnorrSignature, Address, u16),
        >,
    ),
}

pub struct ProposalTopic<TValidatorNetwork> {
    _phantom: PhantomData<TValidatorNetwork>,
}

impl<TValidatorNetwork: ValidatorNetwork + 'static> Topic for ProposalTopic<TValidatorNetwork>
where
    PubsubId<TValidatorNetwork>: std::fmt::Debug + Unpin,
{
    type Item = SignedProposal;

    const BUFFER_SIZE: usize = 8;
    const NAME: &'static str = "tendermint-proposal";
    const VALIDATE: bool = true;
    const MAX_MESSAGES: u32 = 10;
}

/// Pretty much just a wrapper for tendermint, doing some type conversions.
pub(crate) struct ProduceMacroBlock<TValidatorNetwork: ValidatorNetwork + 'static>
where
    PubsubId<TValidatorNetwork>: std::fmt::Debug + Unpin,
{
    tendermint: BoxStream<'static, MappedReturn<TValidatorNetwork>>,
}

impl<TValidatorNetwork: ValidatorNetwork + 'static> ProduceMacroBlock<TValidatorNetwork>
where
    PubsubId<TValidatorNetwork>: std::fmt::Debug + Unpin,
{
    pub fn new(
        blockchain: Arc<RwLock<Blockchain>>,
        network: Arc<TValidatorNetwork>,
        block_producer: BlockProducer,
        validator_slot_band: u16,
        current_validators: Validators,
        network_id: NetworkId,
        block_height: u32,
        state_opt: Option<MacroState>,
        proposal_stream: BoxStream<
            'static,
            SignedProposalMessage<
                Header<PubsubId<TValidatorNetwork>>,
                (SchnorrSignature, Address, u16),
            >,
        >,
        on_double_proposal: Arc<dyn Fn(DoubleProposalProof) + Send + Sync>,
    ) -> Self {
        let input = network
            .receive::<TendermintUpdate>()
            .filter_map(move |(item, validator_id)| async move {
                // Check that the update is for the correct block.
                (item.block_number == block_height).then(|| {
                    let TaggedAggregationMessage { tag, aggregation } = item.message;
                    TaggedAggregationMessage {
                        tag,
                        aggregation: AggregateMessage(aggregation.into_level_update(validator_id)),
                    }
                })
            })
            .boxed();

        let observe_valid_requested_proposal = {
            let double_proposal_detector =
                Arc::new(Mutex::new(DoubleProposalDetector::new(block_height)));
            Arc::new(
                move |address: Address,
                      proposal: TendermintProposal<MacroHeader>,
                      signature: SchnorrSignature| {
                    if let Some(proof) = double_proposal_detector
                        .lock()
                        .observe_valid_proposal(address, proposal, signature)
                    {
                        on_double_proposal(proof);
                    }
                },
            )
        };

        let observe = Arc::clone(&observe_valid_requested_proposal);
        let proposal_stream = proposal_stream
            .inspect(move |proposal| {
                observe(
                    proposal.signature.1.clone(),
                    TendermintProposal {
                        proposal: proposal.message.proposal.0.clone(),
                        round: proposal.message.round,
                        valid_round: proposal.message.valid_round,
                    },
                    proposal.signature.0.clone(),
                )
            })
            .boxed();

        let dependencies = TendermintProtocol::new(
            blockchain,
            network,
            block_producer,
            current_validators,
            validator_slot_band,
            network_id,
            block_height,
            observe_valid_requested_proposal,
        );

        // create the Tendermint instance, which implements Stream
        let tendermint = Tendermint::new(
            dependencies,
            state_opt.and_then(|s| s.into_tendermint_state(block_height)),
            proposal_stream,
            input,
        )
        // and map the return value such that a state update can be persisted.
        .map(move |item| match item {
            TendermintReturn::Decision(decision) => MappedReturn::Decision(decision),
            TendermintReturn::Update(state) => {
                MappedReturn::Update(MacroState::from_tendermint_state(block_height, state))
            }
            TendermintReturn::ProposalAccepted(proposal) => {
                MappedReturn::ProposalAccepted(proposal)
            }
            TendermintReturn::ProposalIgnored(proposal) => MappedReturn::ProposalIgnored(proposal),
            TendermintReturn::ProposalRejected(proposal) => {
                MappedReturn::ProposalRejected(proposal)
            }
        });

        // Create the instance and return it.
        Self {
            tendermint: Box::pin(tendermint),
        }
    }
}

impl<TValidatorNetwork: ValidatorNetwork + 'static> Stream for ProduceMacroBlock<TValidatorNetwork>
where
    PubsubId<TValidatorNetwork>: std::fmt::Debug + Unpin,
{
    type Item = MappedReturn<TValidatorNetwork>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.tendermint.poll_next_unpin(cx)
    }
}
