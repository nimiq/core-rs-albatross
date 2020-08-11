use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use futures::future::BoxFuture;
use futures::task::{Context, Poll};
use futures::{Future, Stream};
use tokio::time;

use block_albatross::{MicroBlock, ViewChange, ViewChangeProof};
use blockchain_albatross::Blockchain;

pub(crate) enum ProduceMicroBlockEvent {
    MicroBlock(MicroBlock),
    ViewChange(ViewChangeProof),
}

pub(crate) struct ProduceMicroBlock {
    blockchain: Arc<Blockchain>,
    signing_key: bls::KeyPair,
    block_number: u32,
    view_number: u32,
    view_change_delay: time::Delay,
    view_change_future: Option<BoxFuture<'static, ViewChangeProof>>,
}

impl ProduceMicroBlock {
    pub fn new(
        blockchain: Arc<Blockchain>,
        signing_key: bls::KeyPair,
        block_number: u32,
        view_number: u32,
        view_change_delay: Duration,
    ) -> Self {
        Self {
            blockchain,
            block_number,
            view_number,
            view_change_delay: time::delay_for(view_change_delay),
            view_change_future: None,
        }
    }

    fn start_view_change(&self) -> impl Future<Output = ViewChangeProof> {
        let view_change = ViewChange {
            block_number: self.block_number,
            new_view_number: self.view_number + 1,
            prev_seed: Default::default(),
        };
    }
}

impl Stream for ProduceMicroBlock {
    type Item = ProduceMicroBlockEvent;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        // If we are the block proposer, generate the block and return.
        // TODO optimize, don't check on every poll

        let foo = self
            .blockchain
            .get_slot_at(self.block_number, self.view_number, None)
            .expect("Can't determine slot");

        // Wait for the view change timer to elapse.
        if !self.view_change_delay.is_elapsed() {
            if let Poll::Pending = self.view_change_delay.poll(cx) {
                return Poll::Pending;
            }
        }

        //

        Poll::Pending
    }
}
