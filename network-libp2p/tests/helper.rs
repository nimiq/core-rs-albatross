use futures::{Stream, StreamExt};
use nimiq_network_interface::network::{NetworkEvent, SubscribeEvents};
use nimiq_network_libp2p::PeerId;

pub fn assert_peer_joined(event: &NetworkEvent<PeerId>, wanted_peer_id: &PeerId) {
    if let NetworkEvent::PeerJoined(peer_id, _) = event {
        assert_eq!(peer_id, wanted_peer_id);
    } else {
        panic!("Event is not a NetworkEvent::PeerJoined: {:?}", event);
    }
}

pub fn assert_peer_left(event: &NetworkEvent<PeerId>, wanted_peer_id: &PeerId) {
    if let NetworkEvent::PeerLeft(peer_id) = event {
        assert_eq!(peer_id, wanted_peer_id);
    } else {
        panic!("Event is not a NetworkEvent::PeerLeft: {:?}", event);
    }
}

pub async fn get_next_peer_event(events: &mut SubscribeEvents<PeerId>) -> NetworkEvent<PeerId> {
    while let Ok(event) = events.next().await.unwrap() {
        match event {
            NetworkEvent::DhtBootstrapped => {}
            _ => return event,
        }
    }
    panic!("No more events");
}
