use libp2p::{multiaddr::Protocol, Multiaddr};

/// Returns true if an address is a secure websocket connection.
/// If address doesn't have the websocket protocol, it will return `false`.
pub fn is_address_ws_secure(address: &Multiaddr) -> bool {
    address.into_iter().any(|p| matches!(p, Protocol::Wss(_)))
}

/// Returns true if every node on the network can decode the protocol, i.e. if multiaddr 0.18,
/// which every release up to v2.2.0 uses, knows it.
///
/// A node that fails to decode an address in a discovery message rejects the whole message and
/// closes the connection to the sender, so addresses with other protocols must never be sent.
/// For example, multiaddr 0.19 added `/http-path`. This is an allowlist so that protocols added by
/// future multiaddr versions are excluded as well.
fn is_protocol_decodable_by_all_peers(protocol: &Protocol) -> bool {
    matches!(
        protocol,
        Protocol::Dccp(_)
            | Protocol::Dns(_)
            | Protocol::Dns4(_)
            | Protocol::Dns6(_)
            | Protocol::Dnsaddr(_)
            | Protocol::Http
            | Protocol::Https
            | Protocol::Ip4(_)
            | Protocol::Ip6(_)
            | Protocol::P2pWebRtcDirect
            | Protocol::P2pWebRtcStar
            | Protocol::WebRTCDirect
            | Protocol::Certhash(_)
            | Protocol::P2pWebSocketStar
            | Protocol::Memory(_)
            | Protocol::Onion(_, _)
            | Protocol::Onion3(_)
            | Protocol::P2p(_)
            | Protocol::P2pCircuit
            | Protocol::Quic
            | Protocol::QuicV1
            | Protocol::Sctp(_)
            | Protocol::Tcp(_)
            | Protocol::Tls
            | Protocol::Noise
            | Protocol::Udp(_)
            | Protocol::Udt
            | Protocol::Unix(_)
            | Protocol::Utp
            | Protocol::WebTransport
            | Protocol::Ws(_)
            | Protocol::Wss(_)
            | Protocol::Ip6zone(_)
            | Protocol::Ipcidr(_)
            | Protocol::Garlic64(_)
            | Protocol::Garlic32(_)
            | Protocol::Sni(_)
            | Protocol::P2pStardust
            | Protocol::WebRTC
    )
}

/// Returns true if every node on the network can decode the address.
/// See [`is_protocol_decodable_by_all_peers`].
pub fn is_address_decodable_by_all_peers(address: &Multiaddr) -> bool {
    address
        .into_iter()
        .all(|p| is_protocol_decodable_by_all_peers(&p))
}

/// Returns the address up to its first protocol that not every node on the network can decode.
/// See [`is_protocol_decodable_by_all_peers`].
pub fn decodable_address_prefix(address: &Multiaddr) -> Multiaddr {
    address
        .into_iter()
        .take_while(is_protocol_decodable_by_all_peers)
        .collect()
}

#[cfg(test)]
mod tests {
    use libp2p::Multiaddr;

    use super::*;

    #[test]
    fn decodable_address_prefix_strips_undecodable_protocols() {
        let address: Multiaddr = "/dns4/seed.local/tcp/443/wss".parse().unwrap();
        assert!(is_address_decodable_by_all_peers(&address));
        assert_eq!(decodable_address_prefix(&address), address);

        let quic: Multiaddr = "/ip4/203.0.113.7/udp/443/quic-v1".parse().unwrap();
        assert!(is_address_decodable_by_all_peers(&quic));

        let http_path: Multiaddr = "/dns4/seed.local/tcp/443/wss/http-path/nimiq"
            .parse()
            .unwrap();
        assert!(!is_address_decodable_by_all_peers(&http_path));
        assert_eq!(decodable_address_prefix(&http_path), address);
    }
}
