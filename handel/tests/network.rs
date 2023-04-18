use std::{sync::Arc, task::Context};

use futures::{FutureExt, StreamExt};
use parking_lot::Mutex;
use tokio::time;

use beserial::{Deserialize, Serialize};
use nimiq_collections::BitSet;
use nimiq_test_log::test;

use nimiq_handel::{
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
struct Net(pub Arc<Mutex<Vec<(LevelUpdate<<Self as Network>::Contribution>, usize)>>>);
impl Network for Net {
    type Contribution = Contribution;
    fn send_to(
        &self,
        msg: (LevelUpdate<Self::Contribution>, usize),
    ) -> futures::future::BoxFuture<'static, ()> {
        self.0.lock().push(msg);

        async move { time::sleep(time::Duration::from_millis(100)).await }.boxed()
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
    time::sleep(time::Duration::from_millis(110)).await;

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
    time::sleep(time::Duration::from_millis(150)).await;
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
