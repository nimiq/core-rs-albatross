use libp2p::{multiaddr::Protocol, Multiaddr};

/// Returns true if an address is a secure websocket connection.
/// If address doesn't have the websocket protocol, it will return `false`.
pub fn is_address_ws_secure(address: &Multiaddr) -> bool {
    address.into_iter().any(|p| matches!(p, Protocol::Wss(_)))
}
