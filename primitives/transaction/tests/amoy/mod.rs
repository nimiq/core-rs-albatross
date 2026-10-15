//! The Polygon Amoy `ChainConfig`, shared by the test binaries that exercise the real program.
//!
//! Lives in its own module rather than in `common` so that binaries which only need the
//! protocol-version helpers do not compile — and warn about — an unused fixture.

use nimiq_serde::Deserialize;
use nimiq_transaction::bridge_contract::ChainConfig;

/// The `source_contract` the pinned blob carries: a stand-in for the address of the
/// `PolygonBridge` deployment. It is the blob's last field, so a deployment's blob differs from
/// the pinned one only in its last 20 bytes.
pub const AMOY_SOURCE_CONTRACT: [u8; 20] = [0xB2; 20];

/// The serialized `ChainConfig` for Polygon Amoy. Redefining it means every bridge instance
/// deployed with the old blob has to be redeployed, and whatever emits it has to be updated to
/// match.
pub const POLYGON_AMOY_CHAIN_CONFIG: &str = "82f10401000000000000000000000000000000000000000000000000000000000000000001010200170000041c0f736f757263655f636f6e747261637400480680c0caf384a3021c06616d6f756e740014041c0e7461726765745f616464726573730028051c0c7461726765745f6e6f6e636500280500ffffffff0f141c116275726e5f626c6f636b5f6865696768740068051c0f7461726765745f636861696e5f6964008801041c0e6272696467655f616464726573732014b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2";

pub fn amoy_chain_config() -> ChainConfig {
    ChainConfig::deserialize_from_vec(&hex::decode(POLYGON_AMOY_CHAIN_CONFIG).unwrap())
        .expect("shipped ChainConfig blob must deserialize")
}
