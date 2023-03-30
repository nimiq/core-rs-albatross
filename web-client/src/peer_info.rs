use tsify::Tsify;

#[cfg(feature = "client")]
use nimiq_network_interface::peer_info::{NodeType, Services};

/// Information about a networking peer.
#[derive(serde::Serialize, Tsify)]
#[serde(rename_all = "camelCase")]
pub struct PlainPeerInfo {
    /// Address of the peer in `Multiaddr` format
    pub address: String,
    /// Node type of the peer
    #[tsify(type = "'full' | 'history' | 'light'")]
    #[serde(rename = "type")]
    pub node_type: String,
}

#[cfg(feature = "client")]
impl PlainPeerInfo {
    pub fn from_native(peer_info: nimiq_network_interface::peer_info::PeerInfo) -> Self {
        let node_type = if peer_info
            .get_services()
            .contains(Services::provided(NodeType::History))
        {
            "history"
        } else if peer_info
            .get_services()
            .contains(Services::provided(NodeType::Full))
        {
            "full"
        } else {
            "light"
        };

        Self {
            address: peer_info.get_address().to_string(),
            node_type: node_type.to_string(),
        }
    }
}
