use std::time::Duration;

use super::network::MockNetwork;
use async_trait::async_trait;
use futures::{Stream, StreamExt};
use std::pin::Pin;

use nimiq_network_interface::message::Message;
use nimiq_network_interface::network::Network;
use nimiq_network_interface::peer::{Peer, SendError};
use nimiq_validator_network_interface::{NetworkError, ValidatorNetwork};

pub struct MockValidatorNetwork {
    pub mock_network: MockNetwork,
}

impl MockValidatorNetwork {
    /// creates a new MockValidatorNetwork node who identifies itself with `validator_id`
    pub fn new(validator_id: usize) -> Self {
        MockValidatorNetwork {
            mock_network: MockNetwork::new(validator_id),
        }
    }

    /// connect the instance with another instance of MockValidatorNetwork
    pub fn connect(&self, other: &MockValidatorNetwork) {
        self.mock_network.connect(&other.mock_network)
    }
}

#[async_trait]
impl ValidatorNetwork for MockValidatorNetwork {
    /// Sends Message `msg` to Validatrs with ids `validator_ids`.
    /// Returns a Vector of `Result<(), NetworkError>`, with each index `i` indicating wether or not the sending to
    /// validator `validator_ids[i]` has happened or not.
    async fn send_to<M: Message>(&self, validator_ids: &[usize], msg: &M) -> Vec<Result<(), NetworkError>> {
        // accumulate results
        let mut res: Vec<Result<(), NetworkError>> = vec![];

        for validator in validator_ids {
            // for every validator get the respective peer from the underliying MockNetwork instance
            if let Some(peer) = self.mock_network.get_peer(*validator).await {
                // Try to send `msg` to the peer and push the result into the reuslt accumulation.
                res.push(peer.send(msg).await.map_err(|err| match err {
                    // Transform everyhthing into Unreachable error except for Serialization errors.
                    SendError::Serialization(err) => NetworkError::Serialization(err),
                    _ => NetworkError::Unreachable,
                }));
            } else {
                // If the mockNetwork does not know the peer it is not connected and therefor unreachable.
                res.push(Err(NetworkError::Unreachable))
            }
        }

        res
    }

    /// bradcast a message to all connected validators. If there is no validators connected it should throw an
    /// NetworkError::Offline
    /// TODO
    async fn broadcast<M: Message>(&self, msg: &M) -> Result<(), NetworkError> {
        // TODO this does currently not work and will always return ()
        self.mock_network.broadcast(msg).await;
        if self.mock_network.get_peers().await.len() > 0 {
            Ok(())
        } else {
            Err(NetworkError::Offline)
        }
    }

    /// Receive messages of type `M` only from the specified set of validators `validator_ids`.
    fn receive_from<M: Message>(&self, _validator_ids: &[usize]) -> Pin<Box<dyn Stream<Item = (M, usize)> + Send>> {
        // select over all streams for every validator_id in validator_ids
        /*stream::select_all(validator_ids.iter().map(|id| {
            // get the corresponding peer from the underlying MockNetwork
            let peer = self.mock_network.get_peer(id).expect("Peer does not exist");

            let validator_id = id.clone();
            peer.receive::<M>().map(move |item| (item, validator_id))
        }))
        .boxed()*/
        todo!()
    }

    /// Receive messages of type `M` from all connected peers.
    /// TODO add connecting validators
    fn receive<M: Message>(&self) -> Pin<Box<dyn Stream<Item = (M, usize)>>> {
        self.mock_network
            .receive_from_all::<M>()
            // .map(map_fn::<M>)
            .map(|item| (item.0, item.1.id() as usize))
            .boxed()
    }

    /// Reserves a `buffer_size` bytes long cache to hold messages of type `M`
    /// even though they have not yet been registered to be received.
    fn cache<M: Message>(&self, buffer_size: usize, lifetime: Duration) {
        // TODO
        (buffer_size, lifetime); // no linter
    }
}

#[cfg(test)]
mod tests {
    use beserial::{Deserialize, Serialize};
    use nimiq_network_interface::message::Message;
    use nimiq_network_interface::network::Network;
    use nimiq_network_interface::peer::Peer;
    use std::time::Duration;
    use tokio::time::timeout;

    use nimiq_validator_network_interface::ValidatorNetwork;

    use crate::validator_network::MockValidatorNetwork;
    use futures::StreamExt;

    #[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
    struct TestMessage {
        msg: u8,
    }

    impl Message for TestMessage {
        const TYPE_ID: u64 = 7;
    }

    #[tokio::test]
    async fn two_validator_networks_can_connect() {
        let net1 = MockValidatorNetwork::new(1);
        let net2 = MockValidatorNetwork::new(2);
        net1.connect(&net2);

        assert_eq!(net1.mock_network.get_peers().await.len(), 1);
        assert_eq!(net2.mock_network.get_peers().await.len(), 1);

        assert_eq!(net1.mock_network.get_peers().await.first().unwrap().id(), 2);
        assert_eq!(net2.mock_network.get_peers().await.first().unwrap().id(), 1);
    }

    // #[tokio::test]
    // async fn droped_validator_results_in_broadcast_failing() {
    //     let msg = TestMessage { msg: 9 };

    //     let net1 = MockValidatorNetwork::new(1);
    //     let net2 = MockValidatorNetwork::new(2);

    //     net1.connect(&net2);

    //     drop(net2);

    //     net1.broadcast(&msg).await.expect_err("Message could be send, but it should not have.");
    // }

    #[tokio::test]
    async fn two_validators_can_send_to_receive() {
        let original_msg = TestMessage { msg: 9 };
        let om = original_msg.clone();

        let net1 = MockValidatorNetwork::new(1);
        let net2 = MockValidatorNetwork::new(2);

        net1.connect(&net2);

        tokio::spawn(async move {
            if let Some(Err(_)) = net2.send_to(&[1], &original_msg).await.get(0) {
                panic!("send_to failed");
            }
        });

        let msg = net1.receive::<TestMessage>().next().await;
        assert_eq!(om, msg.unwrap().0);
    }

    #[tokio::test]
    async fn two_validators_can_send_to_receive_from() {
        let original_msg = TestMessage { msg: 9 };
        let om = original_msg.clone();

        let net1 = MockValidatorNetwork::new(1);
        let net2 = MockValidatorNetwork::new(2);

        net1.connect(&net2);

        tokio::spawn(async move {
            // make sure on sender side everyting went amoothly
            let send_result = net2.send_to(&[1], &original_msg).await;
            // there must be exactly one result in the returned Vec
            assert_eq!(send_result.len(), 1);

            // the result must not be Err as the 2 networks are connected
            if let Some(Err(_)) = send_result.get(0) {
                panic!("send_to failed");
            }
        });

        let msg = net1.receive_from::<TestMessage>(&[2]).next().await;
        assert_eq!(om, msg.unwrap().0);
    }

    #[tokio::test]
    async fn validators_will_not_receive_from_unrequested() {
        let original_msg1 = TestMessage { msg: 9 };
        let om1 = original_msg1.clone();
        let original_msg2 = TestMessage { msg: 12 };
        let om2 = original_msg2.clone();
        let om21 = original_msg2.clone();

        let net1 = MockValidatorNetwork::new(1);
        let net2 = MockValidatorNetwork::new(2);
        let net3 = MockValidatorNetwork::new(3);
        let net4 = MockValidatorNetwork::new(4);
        net1.connect(&net2);
        net1.connect(&net3);
        net1.connect(&net4);
        net2.connect(&net3);
        net2.connect(&net4);
        net3.connect(&net4);

        tokio::spawn(async move {
            // send message to net1 and net4
            if let Some(Err(_)) = net2.send_to(&[1, 4], &original_msg1).await.get(0) {
                panic!("send_to failed");
            }
        });

        tokio::spawn(async move {
            // send message to net1 and net4
            if let Some(Err(_)) = net3.send_to(&[1, 4], &original_msg2).await.get(0) {
                panic!("send_to failed");
            }
        });

        tokio::spawn(async move {
            // net4 must receive 2 messages one from net2 as well as one from net3
            let mut input = net4.receive_from::<TestMessage>(&[2, 3]);
            let msg1 = input.next().await.unwrap();
            let msg2 = input.next().await.unwrap();
            // make sure both messages were received as requested
            assert!((msg1.0 == om1 && msg2.0 == om2 && msg1.1 == 2 && msg2.1 == 3) || (msg1.0 == om2 && msg2.0 == om1 && msg1.1 == 3 && msg2.1 == 2));
        });

        // net1 must not receive the message from net2, but it also must receive the message from net 3
        let mut input = net1.receive_from::<TestMessage>(&[3]);
        let msg = input.next().await;
        assert_eq!(om21, msg.unwrap().0);
        // wait for a little bit of time to receive another message (which should not arrive!)
        timeout(Duration::from_millis(300), input.next())
            .await
            .expect_err("Received a message which should not have been received!");
    }
}
