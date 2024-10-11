use std::{
    collections::HashMap,
    fmt::Debug,
    future::Future,
    pin::Pin,
    task::{Context, Poll, Waker},
    time::Duration,
};

use futures::{
    future::{BoxFuture, FutureExt},
    stream::StreamExt,
};
use instant::Instant;
use linked_hash_map::{Entry, LinkedHashMap};
use nimiq_collections::BitSet;
use nimiq_utils::{stream::FuturesUnordered, WakerExt as _};

use crate::{contribution::AggregatableContribution, identity::Identity, update::LevelUpdate};

/// Trait defining the interface to the network. The only requirement for handel is that the network is able to send
/// a message to a specific validator.
pub trait Network: Unpin + Send + Sync + 'static {
    type Contribution: AggregatableContribution;
    type Error: Debug + Send;

    /// Sends a level update to the node specified by `node_id`.
    /// The node_id is the same one given by the IdentityRegistry.
    fn send_update(
        &self,
        node_id: u16,
        update: LevelUpdate<Self::Contribution>,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + 'static;

    fn ban_node(&self, node_id: u16) -> impl Future<Output = ()> + Send + 'static;
}

#[derive(Clone)]
struct LastLevelUpdate {
    signers: BitSet,
    sent_at: Instant,
}

/// Struct to facilitate sending multiple messages.
/// It will buffer up to one message per recipient, while maintaining the order of messages.
/// Messages that do not fit the buffer will be dropped.
pub struct NetworkHelper<TNetwork: Network> {
    /// The count of nodes this NetworkHelper is going to interact with. Both the `message_buffer`
    /// and `last_messages` are bound in their size by this value.
    num_nodes: usize,

    /// Buffer for one message per recipient. Second value of the pair is the index of the next
    /// message which needs to be sent after this one. If there is no message buffered, it will
    /// point to `message_buffer.len()` which is the first OOB index.
    message_buffer: LinkedHashMap<usize, LevelUpdate<TNetwork::Contribution>>,

    /// Information about the last message sent to each recipient, used to deduplicate messages.
    last_messages: HashMap<usize, LastLevelUpdate>,

    /// The collection of currently pending send futures.
    pending_sends: FuturesUnordered<BoxFuture<'static, Result<(usize, BitSet), TNetwork::Error>>>,

    /// The collection of currently pending ban futures.
    pending_bans: FuturesUnordered<BoxFuture<'static, ()>>,

    /// The network to send messages to other nodes.
    network: TNetwork,

    /// A waker option which is set whenever the future was called while there was no future pending.
    waker: Option<Waker>,
}

impl<TNetwork: Network> NetworkHelper<TNetwork> {
    const MAX_PARALLEL_SENDS: usize = 10;
    const ALLOW_RESEND_AFTER: Duration = Duration::from_secs(5);

    pub fn new(num_nodes: usize, network: TNetwork) -> Self {
        Self {
            num_nodes,
            message_buffer: LinkedHashMap::with_capacity(num_nodes),
            last_messages: HashMap::with_capacity(num_nodes),
            pending_sends: FuturesUnordered::new(),
            pending_bans: FuturesUnordered::new(),
            network,
            waker: None,
        }
    }

    pub fn send(&mut self, node_id: Identity, msg: LevelUpdate<TNetwork::Contribution>) {
        // Get the index of the node from the identity. This will fail for any
        // identity which is empty or has more than one identity within it.
        // Annoyingly the compiler cannot infer the usize here properly.
        let Ok(node_id): Result<usize, _> = node_id.try_into() else {
            return;
        };

        // `message_buffer` and `last_messages` have the same length, so this check prevents
        // out-of-bounds access to both of these vectors.
        if node_id >= self.num_nodes {
            panic!(
                "Attempted to send to out-of-bounds node_id={}, num_nodes = {}",
                node_id, self.num_nodes,
            );
        }

        // If an update with the same signers was recently sent to node_id, we drop it to avoid
        // unnecessarily repeating the same updates.
        if let Some(last_update) = self.last_messages.get(&node_id) {
            let same_signers = last_update.signers == msg.aggregate.contributors();
            let recently_sent = last_update.sent_at.elapsed() < Self::ALLOW_RESEND_AFTER;
            if same_signers && recently_sent {
                return;
            }
        }

        // Insert the message into the message buffer for the recipient.
        match self.message_buffer.entry(node_id) {
            Entry::Occupied(mut entry) => {
                *entry.get_mut() = msg;
            }
            Entry::Vacant(entry) => {
                entry.insert(msg);
            }
        };

        // Wake as a new message was added.
        self.waker.wake();
    }

    pub fn ban_node(&mut self, node_id: usize) {
        let future = self.network.ban_node(node_id as u16).boxed();
        self.pending_bans.push(future);
    }
}

impl<TNetwork: Network + Unpin> Future for NetworkHelper<TNetwork> {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // Drive pending ban futures.
        while let Poll::Ready(Some(())) = self.pending_bans.poll_next_unpin(cx) {}

        // Loop here such that after polling pending futures there is an opportunity to create new
        // ones before returning.
        loop {
            // Create futures if there is free room and if there are messages to send.
            while self.pending_sends.len() < Self::MAX_PARALLEL_SENDS {
                // Take the first message
                let Some((node_id, update)) = self.message_buffer.pop_front() else {
                    // No more futures are ready to be created.
                    break;
                };

                // Create the future to send the message and add it to the pending futures.
                let signers = update.aggregate.contributors();
                let sender = self.network.send_update(node_id as u16, update);
                let future = async move { sender.await.map(|_| (node_id, signers)) }.boxed();
                self.pending_sends.push(future);
            }

            // Poll the pending level update senders.
            while let Poll::Ready(Some(result)) = self.pending_sends.poll_next_unpin(cx) {
                match result {
                    Ok((node_id, signers)) => {
                        self.last_messages.insert(
                            node_id,
                            LastLevelUpdate {
                                signers,
                                sent_at: Instant::now(),
                            },
                        );
                    }
                    Err(error) => {
                        // TODO When we fail to send a level update, we should immediately move on
                        //  to the next peer.
                        debug!(?error, "Failed to send level update");
                    }
                };
            }

            // Once there is either the maximum amount of parallel sends or there is no more messages
            // in the buffer, break the loop returning poll pending.
            if self.pending_sends.len() == Self::MAX_PARALLEL_SENDS
                || self.message_buffer.is_empty()
            {
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
    use std::{future, future::Future, sync::Arc, task::Context, time::Duration};

    use futures::FutureExt;
    use nimiq_collections::BitSet;
    use nimiq_test_log::test;
    use parking_lot::Mutex;
    use serde::{Deserialize, Serialize};

    use crate::{
        contribution::{AggregatableContribution, ContributionError},
        identity::Identity,
        network::{Network, NetworkHelper},
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
        type Error = ();

        fn send_update(
            &self,
            node_id: u16,
            update: LevelUpdate<Self::Contribution>,
        ) -> impl Future<Output = Result<(), Self::Error>> + Send + 'static {
            self.0.lock().push((update, node_id));

            async move {
                nimiq_time::sleep(Duration::from_millis(100)).await;
                Ok(())
            }
        }

        fn ban_node(&self, _node_id: u16) -> impl Future<Output = ()> + Send + 'static {
            future::ready(())
        }
    }

    #[test(tokio::test)]
    async fn it_buffers_messages() {
        let t = Arc::new(Mutex::new(vec![]));
        let nw = Net(Arc::clone(&t));
        let mut sender = NetworkHelper::new(20, nw);

        fn send(sender: &mut NetworkHelper<Net>, i: u32) {
            // Actual values do not matter here.
            sender.send(
                Identity::single(i as usize),
                LevelUpdate::new(Contribution(i), None, 0, 0),
            );
        }

        fn poll(sender: &mut NetworkHelper<Net>) {
            let waker = futures::task::noop_waker();
            let mut cx = Context::from_waker(&waker);
            assert!(sender.poll_unpin(&mut cx).is_pending());
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
