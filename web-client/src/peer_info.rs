use wasm_bindgen::prelude::*;

use nimiq_network_interface::peer_info::{NodeType, Services};

/// Information about a networking peer.
#[wasm_bindgen]
pub struct PeerInfo {
    /// Address of the peer in `Multiaddr` format
    #[wasm_bindgen(getter_with_clone, readonly)]
    pub address: String,
    /// Type of node (`'full' | 'history' | 'light'`)
    #[wasm_bindgen(getter_with_clone, readonly, js_name = "type")]
    pub node_type: String,
}

impl PeerInfo {
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
