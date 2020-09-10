use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use futures::future::BoxFuture;
use futures::task::{Context, Poll};
use futures::{ready, Future, FutureExt, Stream};
use tokio::time;

use block_albatross::{ForkProof, MicroBlock, SignedViewChange, ViewChange, ViewChangeProof};
use block_production_albatross::BlockProducer;
use blockchain_albatross::Blockchain;
use mempool::Mempool;
use utils::time::systemtime_to_timestamp;
use vrf::VrfSeed;

use crate::validator2::mock::ViewChangeHandel;

pub(crate) enum ProduceMicroBlockEvent {
    MicroBlock(MicroBlock),
    ViewChange(u32, ViewChangeProof),
}

#[derive(Clone)]
struct NextProduceMicroBlockEvent {
    blockchain: Arc<Blockchain>,
    mempool: Arc<Mempool>,
    signing_key: bls::KeyPair,
    validator_id: u16,
    fork_proofs: Vec<ForkProof>,
    view_number: u32,
    view_change_proof: Option<ViewChangeProof>,
    view_change_delay: Duration,
    block_number: u32,
    prev_seed: VrfSeed,
}

impl NextProduceMicroBlockEvent {
    fn new(
        blockchain: Arc<Blockchain>,
        mempool: Arc<Mempool>,
        signing_key: bls::KeyPair,
        validator_id: u16,
        fork_proofs: Vec<ForkProof>,
        view_number: u32,
        view_change_proof: Option<ViewChangeProof>,
        view_change_delay: Duration,
    ) -> Self {
        let (block_number, prev_seed) = {
            let head = blockchain.head();
            (head.block_number() + 1, head.seed().clone())
        };

        Self {
            blockchain,
            mempool,
            signing_key,
            validator_id,
            fork_proofs,
            view_number,
            view_change_proof,
            view_change_delay,
            block_number,
            prev_seed,
        }
    }

    async fn next(mut self) -> (ProduceMicroBlockEvent, NextProduceMicroBlockEvent) {
        let event = if self.is_our_turn() {
            ProduceMicroBlockEvent::MicroBlock(self.produce_micro_block())
        } else {
            time::delay_for(self.view_change_delay).await;
            let (new_view_number, view_change_proof) = self.change_view().await;
            self.view_number = new_view_number;
            self.view_change_proof = Some(view_change_proof.clone());
            ProduceMicroBlockEvent::ViewChange(new_view_number, view_change_proof)
        };
        (event, self)
    }

    fn is_our_turn(&self) -> bool {
        let (slot, _) =
            self.blockchain
                .get_slot_owner_at(self.block_number, self.view_number, None);
        &self.signing_key.public_key.compress() == slot.validator_slot.public_key().compressed()
    }

    fn produce_micro_block(&self) -> MicroBlock {
        let producer = BlockProducer::new(
            Arc::clone(&self.blockchain),
            Arc::clone(&self.mempool),
            self.signing_key.clone(),
        );

        let _lock = self.blockchain.lock();
        let timestamp = u64::max(
            self.blockchain.head().header().timestamp(),
            systemtime_to_timestamp(SystemTime::now()),
        );
        producer.next_micro_block(
            timestamp,
            self.view_number,
            self.view_change_proof.clone(),
            self.fork_proofs.clone(),
            vec![], // TODO
        )
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
        mempool: Arc<Mempool>,
        signing_key: bls::KeyPair,
        validator_id: u16,
        fork_proofs: Vec<ForkProof>,
        view_number: u32,
        view_change_proof: Option<ViewChangeProof>,
        view_change_delay: Duration,
    ) -> Self {
        let next_event = NextProduceMicroBlockEvent::new(
            blockchain,
            mempool,
            signing_key,
            validator_id,
            fork_proofs,
            view_number,
            view_change_proof,
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
