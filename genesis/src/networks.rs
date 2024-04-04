use std::env;
#[cfg(feature = "genesis-override")]
use std::path::Path;

use nimiq_block::Block;
#[cfg(feature = "genesis-override")]
use nimiq_database::volatile::VolatileDatabase;
#[cfg(feature = "genesis-override")]
use nimiq_genesis_builder::{GenesisBuilder, GenesisBuilderError, GenesisInfo};
use nimiq_hash::Blake2bHash;
pub use nimiq_primitives::networks::NetworkId;
use nimiq_primitives::trie::TrieItem;
use nimiq_serde::Deserialize;
#[cfg(feature = "genesis-override")]
use nimiq_serde::Serialize;

#[derive(Clone, Debug)]
struct GenesisData {
    block: &'static [u8],
    hash: Blake2bHash,
    accounts: &'static [u8],
}

#[derive(Clone, Debug)]
pub struct NetworkInfo {
    network_id: NetworkId,
    name: &'static str,
    genesis: GenesisData,
}

impl NetworkInfo {
    #[inline]
    pub fn network_id(&self) -> NetworkId {
        self.network_id
    }

    #[inline]
    pub fn name(&self) -> String {
        self.name.into()
    }

    #[inline]
    pub fn genesis_block(&self) -> Block {
        Deserialize::deserialize_from_vec(self.genesis.block)
            .expect("Failed to deserialize genesis block.")
    }

    #[inline]
    pub fn genesis_hash(&self) -> &Blake2bHash {
        &self.genesis.hash
    }

    #[inline]
    pub fn genesis_accounts(&self) -> Vec<TrieItem> {
        Deserialize::deserialize_from_vec(self.genesis.accounts)
            .expect("Failed to deserialize genesis accounts.")
    }

    pub fn from_network_id(network_id: NetworkId) -> &'static Self {
        network(network_id).unwrap_or_else(|| panic!("No such network ID: {network_id}"))
    }
}

#[cfg(feature = "genesis-override")]
fn read_genesis_config(config: &Path) -> Result<GenesisData, GenesisBuilderError> {
    let env = VolatileDatabase::new(20).expect("Could not open a volatile database");

    let GenesisInfo {
        block,
        hash,
        accounts,
    } = GenesisBuilder::from_config_file(config)?.generate(env)?;

    let block = block.serialize_to_vec();
    let accounts = accounts.serialize_to_vec();

    Ok(GenesisData {
        block: Box::leak(block.into_boxed_slice()),
        hash,
        accounts: Box::leak(accounts.into_boxed_slice()),
    })
}

fn network(network_id: NetworkId) -> Option<&'static NetworkInfo> {
    let result = network_impl(network_id);
    if let Some(info) = result {
        assert_eq!(network_id, info.network_id);
        assert_eq!(network_id, info.genesis_block().network());
    }
    result
}

fn network_impl(network_id: NetworkId) -> Option<&'static NetworkInfo> {
    Some(match network_id {
        NetworkId::DevAlbatross => {
            #[cfg(feature = "genesis-override")]
            {
                use std::sync::OnceLock;
                static OVERRIDE: OnceLock<Option<NetworkInfo>> = OnceLock::new();
                if let Some(info) = OVERRIDE.get_or_init(|| {
                    let override_path = env::var_os("NIMIQ_OVERRIDE_DEVNET_CONFIG");
                    override_path.map(|p| NetworkInfo {
                        network_id: NetworkId::DevAlbatross,
                        name: "dev-albatross",
                        genesis: read_genesis_config(Path::new(&p))
                            .expect("failure reading provided NIMIQ_OVERRIDE_DEVNET_CONFIG"),
                    })
                }) {
                    return Some(info);
                }
            }
            static INFO: NetworkInfo = NetworkInfo {
                network_id: NetworkId::DevAlbatross,
                name: "dev-albatross",
                genesis: include!(concat!(
                    env!("OUT_DIR"),
                    "/genesis/dev-albatross/genesis.rs",
                )),
            };
            &INFO
        }
        NetworkId::TestAlbatross => {
            #[cfg(feature = "genesis-override")]
            {
                use std::sync::OnceLock;
                static OVERRIDE: OnceLock<Option<NetworkInfo>> = OnceLock::new();
                if let Some(info) = OVERRIDE.get_or_init(|| {
                    let override_path = env::var_os("NIMIQ_OVERRIDE_TESTNET_CONFIG");
                    override_path.map(|p| NetworkInfo {
                        network_id: NetworkId::TestAlbatross,
                        name: "test-albatross",
                        genesis: read_genesis_config(Path::new(&p))
                            .expect("failure reading provided NIMIQ_OVERRIDE_TESTNET_CONFIG"),
                    })
                }) {
                    return Some(info);
                }
            }
            static INFO: NetworkInfo = NetworkInfo {
                network_id: NetworkId::TestAlbatross,
                name: "test-albatross",
                genesis: include!(concat!(
                    env!("OUT_DIR"),
                    "/genesis/test-albatross/genesis.rs"
                )),
            };
            &INFO
        }
        NetworkId::UnitAlbatross => {
            static INFO: NetworkInfo = NetworkInfo {
                network_id: NetworkId::UnitAlbatross,
                name: "unit-albatross",
                genesis: include!(concat!(
                    env!("OUT_DIR"),
                    "/genesis/unit-albatross/genesis.rs"
                )),
            };
            &INFO
        }
        NetworkId::Main => {
            #[cfg(feature = "genesis-override")]
            {
                use std::sync::OnceLock;
                static OVERRIDE: OnceLock<Option<NetworkInfo>> = OnceLock::new();
                if let Some(info) = OVERRIDE.get_or_init(|| {
                    let override_path = env::var_os("NIMIQ_OVERRIDE_MAINET_CONFIG");
                    override_path.map(|p| NetworkInfo {
                        network_id: NetworkId::MainAlbatross,
                        name: "main-albatross",
                        genesis: read_genesis_config(Path::new(&p))
                            .expect("failure reading provided NIMIQ_OVERRIDE_MAINET_CONFIG"),
                    })
                }) {
                    return Some(info);
                }
            }
            static INFO: NetworkInfo = NetworkInfo {
                network_id: NetworkId::MainAlbatross,
                name: "main-albatross",
                genesis: include!(concat!(
                    env!("OUT_DIR"),
                    "/genesis/main-albatross/genesis.rs"
                )),
            };
            &INFO
        }
        _ => return None,
    })
}
