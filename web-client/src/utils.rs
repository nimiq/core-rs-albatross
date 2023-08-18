use nimiq_primitives::networks::NetworkId;
use wasm_bindgen::JsError;

/// Convert a NetworkId to u8
pub fn from_network_id(network_id: NetworkId) -> u8 {
    match network_id {
        NetworkId::Test => 1,
        NetworkId::Dev => 2,
        NetworkId::Bounty => 3,
        NetworkId::Dummy => 4,
        NetworkId::Main => 42,

        NetworkId::TestAlbatross => 5,
        NetworkId::DevAlbatross => 6,
        NetworkId::UnitAlbatross => 7,
        NetworkId::MainAlbatross => 24,
    }
}

/// Convert u8 to a NetworkId
pub fn to_network_id(network_id: u8) -> Result<NetworkId, JsError> {
    match network_id {
        1 => Ok(NetworkId::Test),
        2 => Ok(NetworkId::Dev),
        3 => Ok(NetworkId::Bounty),
        4 => Ok(NetworkId::Dummy),
        42 => Ok(NetworkId::Main),

        5 => Ok(NetworkId::TestAlbatross),
        6 => Ok(NetworkId::DevAlbatross),
        7 => Ok(NetworkId::UnitAlbatross),
        24 => Ok(NetworkId::MainAlbatross),

        _ => Err(JsError::new("Unknown network ID")),
    }
}
