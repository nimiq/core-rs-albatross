use std::marker::PhantomData;
use std::pin::Pin;
use std::sync::Arc;

use futures::future::{BoxFuture, FutureExt};
use futures::sink::Sink;
use futures::task::{Context, Poll};

use nimiq_network_interface::message::Message;
use nimiq_validator_network::ValidatorNetwork;

// TODO:
// * future per peer.
// * one message to multiple peers

struct SendingFuture<N: ValidatorNetwork> {
    network: Arc<N>,
}

impl<N: ValidatorNetwork> SendingFuture<N> {
    pub async fn send<M: Message + Unpin + std::fmt::Debug>(self, msg: (M, usize)) {
        let result = self.network.send_to(&[msg.1], &msg.0).await;
        if let Some(Err(err)) = result.get(0) {
            debug!("Sending msg: {:?} to validator #{} failed: {:?}", &msg.0, &msg.1, err);
        }
    }
}

/// Implementation of a simple Sink Wrapper for the NetworkInterface's Network trait
pub struct NetworkSink<M: Message + Unpin, N: ValidatorNetwork> {
    /// The network this sink is sending its messages over
    network: Arc<N>,
    /// The currently executed future of sending an item.
    current_future: Option<BoxFuture<'static, ()>>,

    phantom: PhantomData<M>,
}

impl<M: Message + Unpin, N: ValidatorNetwork> NetworkSink<M, N> {
    pub fn new(network: Arc<N>) -> Self {
        Self {
            network,
            current_future: None,
            phantom: PhantomData,
        }
    }
}

impl<M: Message + Unpin + std::fmt::Debug, N: ValidatorNetwork + 'static> Sink<(M, usize)> for NetworkSink<M, N> {
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
            let fut = (SendingFuture { network: self.network.clone() }).send(item).boxed();
            self.current_future = Some(fut);
            Ok(())
        }
    }
}
