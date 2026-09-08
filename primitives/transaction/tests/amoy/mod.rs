//! The Polygon Amoy `ChainConfig`, shared by the test binaries that exercise the real program.
//!
//! Lives in its own module rather than in `common` so that binaries which only need the
//! protocol-version helpers do not compile — and warn about — an unused fixture.

use nimiq_serde::Deserialize;
use nimiq_transaction::bridge_contract::ChainConfig;

/// The serialized `ChainConfig` for Polygon Amoy. Redefining it means every bridge instance
/// deployed with the old blob has to be redeployed, and whatever emits it has to be updated to
/// match.
pub const POLYGON_AMOY_CHAIN_CONFIG: &str = "82f104050000000000000000000000000000000000000000000000000000000000000000010102001100340680c0caf384a3021c06616d6f756e740000041c0e7461726765745f616464726573730014051c0c7461726765745f6e6f6e636500140500ffffffff0f141c116275726e5f626c6f636b5f6865696768740054051c0f7461726765745f636861696e5f696420";

pub fn amoy_chain_config() -> ChainConfig {
    ChainConfig::deserialize_from_vec(&hex::decode(POLYGON_AMOY_CHAIN_CONFIG).unwrap())
        .expect("shipped ChainConfig blob must deserialize")
}
