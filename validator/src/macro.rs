use std::{
    marker::PhantomData,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use futures::{
    future,
    stream::{BoxStream, Stream, StreamExt},
};
use nimiq_block::MacroBlock;
use nimiq_blockchain::{BlockProducer, Blockchain};
use nimiq_keys::Signature as SchnorrSignature;
use nimiq_network_interface::network::Topic;
use nimiq_primitives::slots_allocation::Validators;
use nimiq_tendermint::{Return as TendermintReturn, SignedProposalMessage, Tendermint};
use nimiq_validator_network::ValidatorNetwork;
use parking_lot::RwLock;

use crate::{
    aggregation::tendermint::{
        proposal::{Header, SignedProposal},
        state::MacroState,
        update_message::TendermintUpdate,
    },
    tendermint::TendermintProtocol,
};

pub(crate) enum MappedReturn<TValidatorNetwork: ValidatorNetwork + 'static>
where
    <TValidatorNetwork as ValidatorNetwork>::PubsubId: std::fmt::Debug + Unpin,
{
    Update(MacroState),
    Decision(MacroBlock),
    ProposalAccepted(
        SignedProposalMessage<Header<TValidatorNetwork::PubsubId>, (SchnorrSignature, u16)>,
    ),
    ProposalIgnored(
        SignedProposalMessage<Header<TValidatorNetwork::PubsubId>, (SchnorrSignature, u16)>,
    ),
    ProposalRejected(
        SignedProposalMessage<Header<TValidatorNetwork::PubsubId>, (SchnorrSignature, u16)>,
    ),
}

pub struct ProposalTopic<TValidatorNetwork> {
    _phantom: PhantomData<TValidatorNetwork>,
}

impl<TValidatorNetwork: ValidatorNetwork + 'static> Topic for ProposalTopic<TValidatorNetwork>
where
    <TValidatorNetwork as ValidatorNetwork>::PubsubId: std::fmt::Debug + Unpin,
{
    type Item = SignedProposal;

    const BUFFER_SIZE: usize = 8;
    const NAME: &'static str = "tendermint-proposal";
    const VALIDATE: bool = true;
}

/// Pretty much just a wrapper for tendermint, doing some type conversions.
pub(crate) struct ProduceMacroBlock<TValidatorNetwork: ValidatorNetwork + 'static>
where
    <TValidatorNetwork as ValidatorNetwork>::PubsubId: std::fmt::Debug + Unpin,
{
    tendermint: BoxStream<'static, MappedReturn<TValidatorNetwork>>,
}

impl<TValidatorNetwork: ValidatorNetwork + 'static> ProduceMacroBlock<TValidatorNetwork>
where
    <TValidatorNetwork as ValidatorNetwork>::PubsubId: std::fmt::Debug + Unpin,
{
    pub fn new(
        blockchain: Arc<RwLock<Blockchain>>,
        network: Arc<TValidatorNetwork>,
        block_producer: BlockProducer,
        validator_slot_band: u16,
        current_validators: Validators,
        block_height: u32,
        state_opt: Option<MacroState>,
        proposal_stream: BoxStream<
            'static,
            SignedProposalMessage<
                Header<<TValidatorNetwork as ValidatorNetwork>::PubsubId>,
                (SchnorrSignature, u16),
            >,
        >,
    ) -> Self {
        let input = network
            .receive::<TendermintUpdate>()
            .filter_map(move |item| {
                future::ready(if item.0 .1 == block_height {
                    Some(item.0 .0)
                } else {
                    None
                })
            })
            .boxed();

        let dependencies = TendermintProtocol::new(
            blockchain,
            network,
            block_producer,
            current_validators,
            validator_slot_band,
            block_height,
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
    <TValidatorNetwork as ValidatorNetwork>::PubsubId: std::fmt::Debug + Unpin,
{
    type Item = MappedReturn<TValidatorNetwork>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.tendermint.poll_next_unpin(cx)
    }
}
