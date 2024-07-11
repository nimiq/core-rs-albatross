use std::{
    pin::Pin,
    task::{Context, Poll, Waker},
};

use futures::{
    future::{BoxFuture, FutureExt},
    stream::{FuturesUnordered, StreamExt},
    Stream,
};
use nimiq_utils::WakerExt as _;

use crate::{contribution::AggregatableContribution, update::LevelUpdate};

/// Trait defining the interface to the network. The only requirement for handel is that the network is able to send
/// a message to a specific validator.
pub trait Network: Unpin + Send + 'static {
    type Contribution: AggregatableContribution;
    /// Sends message `msg.0` to validator identified by index `msg.1`. The mapping for the identifier is the
    /// same as the one given by the identity register
    fn send_to(&self, msg: (LevelUpdate<Self::Contribution>, u16)) -> BoxFuture<'static, ()>;
}

/// Struct to facilitate sending multiple messages.
/// It will buffer up to one message per recipient, while maintaining the order of messages.
/// Messages that do not fit the buffer will be dropped.
pub struct LevelUpdateSender<TNetwork: Network> {
    /// Buffer for one message per recipient. Second value of the pair is the index of the next message which needs to
    /// be sent after this one. In case there is none it will be set to the len of the Vec, which is an OOB index.
    message_buffer: Vec<Option<(LevelUpdate<TNetwork::Contribution>, usize)>>,

    /// Index of the first message in this buffer that should be sent. If there is no message buffered it will point to
    /// `message_buffer.len()` which is the first OOB index.
    first: usize,

    /// Index of the last message in this buffer that should be sent. It is used to quickly append messages.
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
        self.waker.wake();
    }
}

impl<TNetwork: Network + Unpin> Stream for LevelUpdateSender<TNetwork> {
    type Item = ();

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        // Loop here such that after polling pending futures there is an opportunity to create new ones
        // before returning.
        loop {
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
                let mut fut = self.network.send_to((msg, self.first.try_into().unwrap()));
                // Poll it, and only add it to the pending futures if it is in fact pending.
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
                // Nothing to do here. Message was send.
            }
            // Once there is either the maximum amount of parallel sends or there is no more messages
            // in the buffer, break the loop returning poll pending.
            if self.pending.len() == Self::MAX_PARALLEL_SENDS || self.first == len {
                // Store a waker in case a message is added.
                // This is necessary as this future will always return `Poll::Pending`, while the
                // buffer might be empty and the pending futures might be empty as well.
                self.waker.store_waker(cx);

                // Return `Poll::Pending` as this stream is neither allowed to produce items nor to terminate.
                return Poll::Pending;
            }
        }
    }
}

#[cfg(test)]
mod test {
    use std::{sync::Arc, task::Context, time::Duration};

    use futures::{FutureExt, StreamExt};
    use nimiq_collections::BitSet;
    use nimiq_test_log::test;
    use parking_lot::Mutex;
    use serde::{Deserialize, Serialize};

    use crate::{
        contribution::{AggregatableContribution, ContributionError},
        network::{LevelUpdateSender, Network},
        update::LevelUpdate,
    };

    /// Super Basic and dysfunctional Contribution. Only needed to use it with the network messages.
    #[derive(Clone, Debug, Deserialize, Serialize)]
    struct Contribution(pub u32);
    impl AggregatableContribution for Contribution {
        fn contributors(&self) -> BitSet {
            // irrelevant
            BitSet::default()
        }
        fn combine(&mut self, _other_contribution: &Self) -> Result<(), ContributionError> {
            // irrelevant
            Ok(())
        }
    }

    /// Actual values are irrelevant and only used for testing
    #[derive(Debug)]
    struct Net(pub Arc<Mutex<Vec<(LevelUpdate<<Self as Network>::Contribution>, u16)>>>);
    impl Network for Net {
        type Contribution = Contribution;
        fn send_to(
            &self,
            msg: (LevelUpdate<Self::Contribution>, u16),
        ) -> futures::future::BoxFuture<'static, ()> {
            self.0.lock().push(msg);

            async move { nimiq_time::sleep(Duration::from_millis(100)).await }.boxed()
        }
    }

    #[test(tokio::test)]
    async fn it_buffers_messages() {
        let t = Arc::new(Mutex::new(vec![]));
        let nw = Net(Arc::clone(&t));
        let mut sender = LevelUpdateSender::new(20, nw);

        fn send(sender: &mut LevelUpdateSender<Net>, i: u32) {
            // Actual values do not matter here.
            sender.send((LevelUpdate::new(Contribution(i), None, 0, 0), i as usize));
        }

        fn poll(sender: &mut LevelUpdateSender<Net>) {
            let waker = futures::task::noop_waker();
            let mut cx = Context::from_waker(&waker);
            assert!(sender.poll_next_unpin(&mut cx).is_pending());
        }

        // Send single msg and see if it is sent
        send(&mut sender, 0);
        poll(&mut sender);
        assert_eq!(1, t.lock().len());

        // Clear the buffer so test starts from scratch
        t.lock().clear();

        // Send some messages (but less than `MAX_PARALLEL_SENDS - 1`) including a duplicate
        send(&mut sender, 0);
        send(&mut sender, 1);
        send(&mut sender, 2);
        send(&mut sender, 4);
        send(&mut sender, 3);
        send(&mut sender, 2); // duplicate
        poll(&mut sender);
        // Make sure the duplicate is not sent
        assert_eq!(5, t.lock().len());
        assert_eq!(0, t.lock()[0].1);
        assert_eq!(1, t.lock()[1].1);
        assert_eq!(2, t.lock()[2].1);
        assert_eq!(4, t.lock()[3].1);
        assert_eq!(3, t.lock()[4].1);

        // Clear the buffer so test starts from scratch
        t.lock().clear();
        // Needed because the send also sleeps
        nimiq_time::sleep(Duration::from_millis(110)).await;

        assert_eq!(0, t.lock().len());
        send(&mut sender, 0);
        send(&mut sender, 1);
        send(&mut sender, 2);
        send(&mut sender, 3);
        send(&mut sender, 4);
        send(&mut sender, 5);
        send(&mut sender, 4); // Duplicate
        send(&mut sender, 6);
        send(&mut sender, 7);
        send(&mut sender, 8);
        send(&mut sender, 9);
        // This is 10 (with the duplicate 11)
        send(&mut sender, 9); // Duplicate these should not be accepted
        send(&mut sender, 8); // Duplicate these should not be accepted
        send(&mut sender, 10);

        // This should produce 10 items, and one pending (the 10)
        poll(&mut sender);
        assert_eq!(10, t.lock().len());

        // Wait for the futures to resolve, imitating a delay
        nimiq_time::sleep(Duration::from_millis(150)).await;
        // Send some more
        send(&mut sender, 9); // Not a Duplicate, this should be accepted
        send(&mut sender, 8); // Not a Duplicate, this should be accepted
        send(&mut sender, 10); // This is a duplicate as 10 is still there from before the last poll

        // Make sure all messages were sent in order.
        poll(&mut sender);
        assert_eq!(13, t.lock().len());
        assert_eq!(0, t.lock()[0].1);
        assert_eq!(1, t.lock()[1].1);
        assert_eq!(2, t.lock()[2].1);
        assert_eq!(3, t.lock()[3].1);
        assert_eq!(4, t.lock()[4].1);
        assert_eq!(5, t.lock()[5].1);
        assert_eq!(6, t.lock()[6].1);
        assert_eq!(7, t.lock()[7].1);
        assert_eq!(8, t.lock()[8].1);
        assert_eq!(9, t.lock()[9].1);
        assert_eq!(10, t.lock()[10].1);
        assert_eq!(9, t.lock()[11].1);
        assert_eq!(8, t.lock()[12].1);
    }
}
