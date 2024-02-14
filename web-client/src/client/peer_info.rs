use nimiq_network_interface::peer_info::{NodeType, Services};
use tsify::Tsify;

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

impl From<nimiq_network_interface::peer_info::PeerInfo> for PlainPeerInfo {
    fn from(peer_info: nimiq_network_interface::peer_info::PeerInfo) -> Self {
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
