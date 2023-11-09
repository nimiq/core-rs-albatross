use std::{
    pin::Pin,
    sync::Arc,
    task::{Context, Poll, Waker},
};

use futures::Stream;
use linked_hash_map::LinkedHashMap;
use nimiq_keys::Signature as SchnorrSignature;
use nimiq_macros::store_waker;
use nimiq_network_interface::network::{Network, PubsubId, Topic};
use nimiq_tendermint::SignedProposalMessage;
use nimiq_validator_network::ValidatorNetwork;
use parking_lot::RwLock;

use super::{aggregation::tendermint::proposal::Header, r#macro::ProposalTopic};

type ProposalAndPubsubId<TValidatorNetwork> = (
    <ProposalTopic<TValidatorNetwork> as Topic>::Item,
    <TValidatorNetwork as ValidatorNetwork>::PubsubId,
);

pub(crate) struct ProposalBuffer<TValidatorNetwork: ValidatorNetwork + 'static>
where
    <TValidatorNetwork as ValidatorNetwork>::PubsubId: std::fmt::Debug + Unpin,
{
    buffer: LinkedHashMap<
        <TValidatorNetwork::NetworkType as Network>::PeerId,
        ProposalAndPubsubId<TValidatorNetwork>,
    >,
    waker: Option<Waker>,
}

impl<TValidatorNetwork: ValidatorNetwork + 'static> ProposalBuffer<TValidatorNetwork>
where
    <TValidatorNetwork as ValidatorNetwork>::PubsubId: std::fmt::Debug + Unpin,
{
    // Ignoring clippy warning: this return type is on purpose
    #[allow(clippy::new_ret_no_self)]
    pub fn new() -> (
        ProposalSender<TValidatorNetwork>,
        ProposalReceiver<TValidatorNetwork>,
    ) {
        let buffer = Self {
            buffer: LinkedHashMap::new(),
            waker: None,
        };
        let shared = Arc::new(RwLock::new(buffer));
        let sender = ProposalSender {
            shared: Arc::clone(&shared),
        };
        let receiver = ProposalReceiver { shared };
        (sender, receiver)
    }
}

pub(crate) struct ProposalSender<TValidatorNetwork: ValidatorNetwork + 'static>
where
    <TValidatorNetwork as ValidatorNetwork>::PubsubId: std::fmt::Debug + Unpin,
{
    shared: Arc<RwLock<ProposalBuffer<TValidatorNetwork>>>,
}

impl<TValidatorNetwork: ValidatorNetwork + 'static> ProposalSender<TValidatorNetwork>
where
    <TValidatorNetwork as ValidatorNetwork>::PubsubId: std::fmt::Debug + Unpin,
{
    pub fn send(&self, proposal: ProposalAndPubsubId<TValidatorNetwork>) {
        let source = proposal.1.propagation_source();
        let mut shared = self.shared.write();
        shared.buffer.insert(source, proposal);
        if let Some(waker) = shared.waker.take() {
            waker.wake()
        }
    }
}

pub(crate) struct ProposalReceiver<TValidatorNetwork: ValidatorNetwork + 'static>
where
    <TValidatorNetwork as ValidatorNetwork>::PubsubId: std::fmt::Debug + Unpin,
{
    shared: Arc<RwLock<ProposalBuffer<TValidatorNetwork>>>,
}

impl<TValidatorNetwork: ValidatorNetwork + 'static> Stream for ProposalReceiver<TValidatorNetwork>
where
    <TValidatorNetwork as ValidatorNetwork>::PubsubId: std::fmt::Debug + Unpin,
{
    type Item = SignedProposalMessage<
        Header<<TValidatorNetwork as ValidatorNetwork>::PubsubId>,
        (SchnorrSignature, u16),
    >;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut shared = self.shared.write();
        if shared.buffer.is_empty() {
            store_waker!(shared, waker, cx);
            Poll::Pending
        } else {
            let value = shared.buffer.pop_front().map(|entry| {
                let (msg, id) = entry.1;
                msg.into_tendermint_signed_message(Some(id))
            });
            Poll::Ready(value)
        }
    }
}

impl<TValidatorNetwork: ValidatorNetwork + 'static> Clone for ProposalReceiver<TValidatorNetwork>
where
    <TValidatorNetwork as ValidatorNetwork>::PubsubId: std::fmt::Debug + Unpin,
{
    fn clone(&self) -> Self {
        Self {
            shared: Arc::clone(&self.shared),
        }
    }
}
