use std::{
    pin::Pin,
    task::{Context, Poll, Waker},
};

use futures::{
    future::{BoxFuture, FutureExt},
    stream::{FuturesUnordered, StreamExt},
    Stream,
};

use nimiq_macros::store_waker;

use crate::{contribution::AggregatableContribution, update::LevelUpdate};

/// Trait defining the interface to the network. The only requirement for handel is that the network is able to send
/// a message to a specific validator.
pub trait Network: Unpin + Send + 'static {
    type Contribution: AggregatableContribution;
    /// Sends message `msg.0` to validator identified by index `msg.1`. The mapping for the identifier is the
    /// same as the one given by the identity register
    fn send_to(&self, msg: (LevelUpdate<Self::Contribution>, usize)) -> BoxFuture<'static, ()>;
}

/// Struct to facilitate sending multiple messages.
/// It will buffer up to one message per recipient, while maintaining the order of messages.
/// Messages that do not fit the buffer will be dropped.
pub struct LevelUpdateSender<TNetwork: Network> {
    /// Buffer for one message per recipient. Second value of the pair is the index of the next message which needs to
    /// be sent after this one. In case there is none it will be set to the len of the Vec, which is an OOB index.
    message_buffer: Vec<Option<(LevelUpdate<TNetwork::Contribution>, usize)>>,

    /// Index of the first message in this buffer that should be send. If there is no message buffered it will point to
    /// `message_buffer.len()` which is the first OOB index.
    first: usize,

    /// Index of the last message in this buffer that should be send. It is used to quickly append messages.
    /// If there is no message buffered it will point to `message_buffer.len()` which is the first OOB index.
    last: usize,

    /// The collection of currently being awaited sending futures.
    pending: FuturesUnordered<BoxFuture<'static, ()>>,

    /// The network used to create the futures for sending a message.
    network: TNetwork,

    /// A waker option which is set whenever the future was called while there was no future pending.
    waker: Option<Waker>,
}

impl<TNetwork: Network> LevelUpdateSender<TNetwork> {
    const MAX_PARALLEL_SENDS: usize = 10;

    pub fn new(len: usize, network: TNetwork) -> Self {
        let message_buffer = vec![None; len];
        Self {
            message_buffer,
            first: len,
            last: len,
            pending: FuturesUnordered::new(),
            network,
            waker: None,
        }
    }

    pub fn send(&mut self, (msg, recipient): (LevelUpdate<TNetwork::Contribution>, usize)) {
        let len = self.message_buffer.len();

        let recipient_buffer = self
            .message_buffer
            .get_mut(recipient)
            .expect("send for recipient must not be out of bounds.");

        if let Some(msg_and_recipient) = recipient_buffer.as_mut() {
            // Preserve the previous ordering if there is one
            msg_and_recipient.0 = msg;
        } else {
            // otherwise add to the end
            *recipient_buffer = Some((msg, len));
            // if there was nothing buffered before, the added message is first and last
            if self.message_buffer.len() == self.first {
                self.first = recipient;
                self.last = recipient;
                // wake
            } else {
                // otherwise the previously last message must be updated to point to the now new last message
                self.message_buffer
                    .get_mut(self.last)
                    .expect(
                        "The last index must not point out of bounds if the buffer is non empty",
                    )
                    .as_mut()
                    .expect("The last index must point to a set buffered message")
                    .1 = recipient;
                // and the last index also needs to be updated.
                self.last = recipient;
            }
        }
        // wake as a new message was added.
        if let Some(waker) = &self.waker {
            waker.wake_by_ref();
        }
    }
}

impl<TNetwork: Network + Unpin> Stream for LevelUpdateSender<TNetwork> {
    type Item = ();

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let len = self.message_buffer.len();
        // create futures if there is free room and if there are messages to send
        while self.first < len && self.pending.len() < Self::MAX_PARALLEL_SENDS {
            let first = self.first;
            // take the first message
            let (msg, next) = self
                .message_buffer
                .get_mut(first)
                .expect("First must not point out of bounds")
                .take()
                .expect("First must point to a valid buffered message");

            // Create the future sending the message
            let mut fut = self.network.send_to((msg, self.first));
            // Poll it, and only add it to then pending futures if it is in fact pending.
            if fut.poll_unpin(cx).is_pending() {
                // If the Future does not resolve immediately add it to the pending futures.
                self.pending.push(fut);
            }

            // Update indices.
            // Note that `next` might be `== len` here in the case of `first == last`
            self.first = next;
            if next == len {
                self.last = len;
            }
        }

        // Keep on polling the futures until they no longer return a result.
        while let Poll::Ready(Some(_)) = self.pending.poll_next_unpin(cx) {
            // nothing to do here. Message was send.
        }

        // store a waker in case a message is added.
        // This is necessary as this future will always return Poll::Pending, while the
        // buffer might be empty and the pending futures might be empty as well.
        store_waker!(self, waker, cx);

        // return Poll::Pending as this stream is neither allowed to produce items nor to terminate.
        Poll::Pending
    }
}
