// Shared by several integration-test binaries; not every one of them uses every item.
#![allow(dead_code)]

use nimiq_primitives::policy::Policy;
use nimiq_serde::Deserialize;
use nimiq_transaction::bridge_contract::ChainConfig;

pub fn for_each_protocol_version(mut f: impl FnMut(u16)) {
    for version in 0..=Policy::max_supported_version() {
        f(version);
    }
}

/// The serialized `ChainConfig` for Polygon Amoy. Redefining it means every bridge instance
/// deployed with the old blob has to be redeployed, and whatever emits it has to be updated to
/// match.
pub const POLYGON_AMOY_CHAIN_CONFIG: &str = "82f104050000000000000000000000000000000000000000000000000000000000000000010102001000340680c0caf384a3021c06616d6f756e740000041c0e7461726765745f616464726573730014051c0c7461726765745f6e6f6e636500140500ffffffff0f141c116275726e5f626c6f636b5f6865696768740082f1041c0f7461726765745f636861696e5f696420";

pub fn amoy_chain_config() -> ChainConfig {
    ChainConfig::deserialize_from_vec(&hex::decode(POLYGON_AMOY_CHAIN_CONFIG).unwrap())
        .expect("pinned Amoy ChainConfig blob must deserialize")
}
