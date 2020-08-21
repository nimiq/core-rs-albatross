use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use futures::future::BoxFuture;
use futures::task::{Context, Poll};
use futures::{ready, Future, FutureExt, Stream};
use tokio::time;

use block_albatross::{MicroBlock, SignedViewChange, ViewChange, ViewChangeProof};
use blockchain_albatross::Blockchain;
use vrf::VrfSeed;

use crate::validator2::mock::ViewChangeHandel;

pub(crate) enum ProduceMicroBlockEvent {
    MicroBlock(MicroBlock),
    ViewChange(u32, ViewChangeProof),
}

#[derive(Clone)]
struct NextProduceMicroBlockEvent {
    blockchain: Arc<Blockchain>,
    signing_key: bls::KeyPair,
    validator_id: u16,
    block_number: u32,
    view_number: u32,
    prev_seed: VrfSeed,
    view_change_delay: Duration,
    last_view_change_proof: Option<ViewChangeProof>,
}

impl NextProduceMicroBlockEvent {
    fn new(
        blockchain: Arc<Blockchain>,
        signing_key: bls::KeyPair,
        validator_id: u16,
        block_number: u32,
        view_number: u32,
        prev_seed: VrfSeed,
        view_change_delay: Duration,
    ) -> Self {
        Self {
            blockchain,
            signing_key,
            validator_id,
            block_number,
            view_number,
            prev_seed,
            view_change_delay,
            last_view_change_proof: None,
        }
    }

    async fn next(mut self) -> (ProduceMicroBlockEvent, NextProduceMicroBlockEvent) {
        let event = if self.is_our_turn() {
            ProduceMicroBlockEvent::MicroBlock(self.propose_micro_block())
        } else {
            time::delay_for(self.view_change_delay).await;
            let (new_view_number, view_change_proof) = self.change_view().await;
            self.view_number = new_view_number;
            self.last_view_change_proof = Some(view_change_proof.clone());
            ProduceMicroBlockEvent::ViewChange(new_view_number, view_change_proof)
        };
        (event, self)
    }

    fn is_our_turn(&self) -> bool {
        let (slot, _) = self
            .blockchain
            .get_slot_at(self.block_number, self.view_number, None)
            .expect("Can't determine slot");
        &self.signing_key.public_key.compress() == slot.validator_slot.public_key().compressed()
    }

    fn propose_micro_block(&self) -> MicroBlock {
        unimplemented!()
    }

    fn change_view(&self) -> impl Future<Output = (u32, ViewChangeProof)> {
        let new_view_number = self.view_number + 1;
        let view_change = ViewChange {
            block_number: self.block_number,
            new_view_number,
            prev_seed: self.prev_seed.clone(),
        };
        let signed_view_change = SignedViewChange::from_message(
            view_change,
            &self.signing_key.secret_key,
            self.validator_id,
        );
        // TODO get at init time??
        let active_validators = self.blockchain.current_validators().clone();
        ViewChangeHandel::new(signed_view_change, self.validator_id, active_validators)
            .map(move |proof| (new_view_number, proof))
    }
}

pub(crate) struct ProduceMicroBlock {
    next_event: Option<BoxFuture<'static, (ProduceMicroBlockEvent, NextProduceMicroBlockEvent)>>,
}

impl ProduceMicroBlock {
    pub fn new(
        blockchain: Arc<Blockchain>,
        signing_key: bls::KeyPair,
        validator_id: u16,
        block_number: u32,
        view_number: u32,
        prev_seed: VrfSeed,
        view_change_delay: Duration,
    ) -> Self {
        let next_event = NextProduceMicroBlockEvent::new(
            blockchain,
            signing_key,
            validator_id,
            block_number,
            view_number,
            prev_seed,
            view_change_delay,
        )
        .next()
        .boxed();
        Self {
            next_event: Some(next_event),
        }
    }
}

impl Stream for ProduceMicroBlock {
    type Item = ProduceMicroBlockEvent;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let next_event = match self.next_event.as_mut() {
            Some(next_event) => next_event,
            None => return Poll::Ready(None),
        };

        let (event, next_event) = ready!(next_event.poll_unpin(cx));
        self.next_event = match &event {
            ProduceMicroBlockEvent::MicroBlock(_) => None,
            ProduceMicroBlockEvent::ViewChange(_, _) => Some(next_event.next().boxed()),
        };
        Poll::Ready(Some(event))
    }
}
