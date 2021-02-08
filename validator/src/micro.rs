use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use futures::future::BoxFuture;
use futures::task::{Context, Poll};
use futures::{ready, FutureExt, Stream};
use tokio::time;

use block_albatross::{ForkProof, MicroBlock, ViewChange, ViewChangeProof};
use block_production_albatross::BlockProducer;
use blockchain_albatross::Blockchain;
use mempool::Mempool;
use nimiq_validator_network::ValidatorNetwork;
use utils::time::systemtime_to_timestamp;
use vrf::VrfSeed;

use crate::aggregation::view_change::ViewChangeAggregation;

pub(crate) enum ProduceMicroBlockEvent {
    MicroBlock(MicroBlock),
    ViewChange(ViewChange, ViewChangeProof),
}

#[derive(Clone)]
struct NextProduceMicroBlockEvent<TValidatorNetwork> {
    blockchain: Arc<Blockchain>,
    mempool: Arc<Mempool>,
    network: Arc<TValidatorNetwork>,
    signing_key: bls::KeyPair,
    validator_id: u16,
    fork_proofs: Vec<ForkProof>,
    view_number: u32,
    view_change_proof: Option<ViewChangeProof>,
    view_change: Option<ViewChange>,
    view_change_delay: Duration,
    block_number: u32,
    prev_seed: VrfSeed,
}

impl<TValidatorNetwork: ValidatorNetwork + 'static> NextProduceMicroBlockEvent<TValidatorNetwork> {
    fn new(
        blockchain: Arc<Blockchain>,
        mempool: Arc<Mempool>,
        network: Arc<TValidatorNetwork>,
        signing_key: bls::KeyPair,
        validator_id: u16,
        fork_proofs: Vec<ForkProof>,
        view_number: u32,
        view_change_proof: Option<ViewChangeProof>,
        view_change: Option<ViewChange>,
        view_change_delay: Duration,
    ) -> Self {
        let (block_number, prev_seed) = {
            let head = blockchain.head();
            (head.block_number() + 1, head.seed().clone())
        };

        Self {
            blockchain,
            mempool,
            network,
            signing_key,
            validator_id,
            fork_proofs,
            view_number,
            view_change_proof,
            view_change,
            view_change_delay,
            block_number,
            prev_seed,
        }
    }

    async fn next(mut self) -> (ProduceMicroBlockEvent, NextProduceMicroBlockEvent<TValidatorNetwork>) {
        let event = if self.is_our_turn() {
            info!("Our turn at #{}:{}, producing micro block", self.block_number, self.view_number);
            ProduceMicroBlockEvent::MicroBlock(self.produce_micro_block())
        } else {
            debug!("Not our turn at #{}:{}, waiting for micro block", self.block_number, self.view_number);
            time::delay_for(self.view_change_delay).await;
            info!(
                "No micro block received within timeout at #{}:{}, starting view change",
                self.block_number, self.view_number
            );
            let (view_change, view_change_proof) = self.change_view().await;
            info!(
                "View change completed for #{}:{}, new view is {}",
                self.block_number, self.view_number, view_change.new_view_number
            );
            self.view_number = view_change.new_view_number;
            self.view_change_proof = Some(view_change_proof.clone());
            ProduceMicroBlockEvent::ViewChange(view_change, view_change_proof)
        };
        (event, self)
    }

    fn is_our_turn(&self) -> bool {
        let (slot, _) = self.blockchain.get_slot_owner_at(self.block_number, self.view_number, None);
        &self.signing_key.public_key.compress() == slot.validator_slot.public_key().compressed()
    }

    fn produce_micro_block(&self) -> MicroBlock {
        let producer = BlockProducer::new(Arc::clone(&self.blockchain), Arc::clone(&self.mempool), self.signing_key.clone());

        let _lock = self.blockchain.lock();
        let timestamp = u64::max(self.blockchain.head().header().timestamp(), systemtime_to_timestamp(SystemTime::now()));
        producer.next_micro_block(
            timestamp,
            self.view_number,
            self.view_change_proof.clone(),
            self.fork_proofs.clone(),
            vec![], // TODO
        )
    }

    async fn change_view(&mut self) -> (ViewChange, ViewChangeProof) {
        let new_view_number = self.view_number + 1;
        let view_change = ViewChange {
            block_number: self.block_number,
            new_view_number,
            prev_seed: self.prev_seed.clone(),
        };

        // Include the previous_view_change_proof only if it has not yet been persisted on chain.
        let view_change_proof = self.view_change.as_ref()
            .map_or(None, |vc| if vc.block_number == self.block_number {
                Some(self.view_change_proof.as_ref().unwrap().sig.clone())
            } else {
                None
            });

        // TODO get at init time?
        let active_validators = self.blockchain.current_validators().clone();
        let (view_change, view_change_proof) = ViewChangeAggregation::start(
            view_change.clone(),
            view_change_proof,
            self.signing_key.clone(),
            self.validator_id,
            active_validators,
            Arc::clone(&self.network),
        )
        .await;

        // set the view change and view_change_proof properties so in case another view change happens they are available.
        self.view_change = Some(view_change.clone());
        self.view_change_proof = Some(view_change_proof.clone());

        (view_change, view_change_proof)
    }
}

pub(crate) struct ProduceMicroBlock<TValidatorNetwork> {
    next_event: Option<BoxFuture<'static, (ProduceMicroBlockEvent, NextProduceMicroBlockEvent<TValidatorNetwork>)>>,
}

impl<TValidatorNetwork: ValidatorNetwork + 'static> ProduceMicroBlock<TValidatorNetwork> {
    pub fn new(
        blockchain: Arc<Blockchain>,
        mempool: Arc<Mempool>,
        network: Arc<TValidatorNetwork>,
        signing_key: bls::KeyPair,
        validator_id: u16,
        fork_proofs: Vec<ForkProof>,
        view_number: u32,
        view_change_proof: Option<ViewChangeProof>,
        view_change: Option<ViewChange>,
        view_change_delay: Duration,
    ) -> Self {
        let next_event = NextProduceMicroBlockEvent::new(
            blockchain,
            mempool,
            network,
            signing_key,
            validator_id,
            fork_proofs,
            view_number,
            view_change_proof,
            view_change,
            view_change_delay,
        )
        .next()
        .boxed();
        Self { next_event: Some(next_event) }
    }
}

impl<TValidatorNetwork: ValidatorNetwork + 'static> Stream for ProduceMicroBlock<TValidatorNetwork> {
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
