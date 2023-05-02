#[cfg(feature = "genesis-override")]
use std::path::Path;
use std::{collections::HashMap, env};

use beserial::Deserialize;
#[cfg(feature = "genesis-override")]
use beserial::{Serialize, SerializeWithLength};
use lazy_static::lazy_static;
use nimiq_block::Block;
#[cfg(feature = "genesis-override")]
use nimiq_database::volatile::VolatileDatabase;
#[cfg(feature = "genesis-override")]
use nimiq_genesis_builder::{GenesisBuilder, GenesisBuilderError, GenesisInfo};
use nimiq_hash::Blake2bHash;
pub use nimiq_primitives::networks::NetworkId;
use nimiq_primitives::trie::TrieItem;

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
        use beserial::DeserializeWithLength;

        DeserializeWithLength::deserialize_from_vec::<u32>(self.genesis.accounts)
            .expect("Failed to deserialize genesis accounts.")
    }

    pub fn from_network_id(network_id: NetworkId) -> &'static Self {
        NETWORK_MAP
            .get(&network_id)
            .unwrap_or_else(|| panic!("No such network ID: {network_id}"))
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
    let accounts = accounts.serialize_to_vec::<u32>();

    Ok(GenesisData {
        block: Box::leak(block.into_boxed_slice()),
        hash,
        accounts: Box::leak(accounts.into_boxed_slice()),
    })
}

lazy_static! {
    static ref NETWORK_MAP: HashMap<NetworkId, NetworkInfo> = {
        let mut m = HashMap::new();
        fn add(m: &mut HashMap<NetworkId, NetworkInfo>, info: NetworkInfo) {
            m.insert(info.network_id, info);
        }

        #[cfg(feature = "genesis-override")]
        let override_path = env::var_os("NIMIQ_OVERRIDE_DEVNET_CONFIG");
        #[cfg(feature = "genesis-override")]
        let dev_genesis = if let Some(p) = override_path {
            read_genesis_config(Path::new(&p))
                .expect("failure reading provided NIMIQ_OVERRIDE_DEVNET_CONFIG")
        } else {
            include!(concat!(
                env!("OUT_DIR"),
                "/genesis/dev-albatross/genesis.rs"
            ))
        };
        #[cfg(not(feature = "genesis-override"))]
        let dev_genesis = include!(concat!(
            env!("OUT_DIR"),
            "/genesis/dev-albatross/genesis.rs"
        ));

        add(
            &mut m,
            NetworkInfo {
                network_id: NetworkId::DevAlbatross,
                name: "dev-albatross",
                genesis: dev_genesis,
            },
        );

        add(
            &mut m,
            NetworkInfo {
                network_id: NetworkId::TestAlbatross,
                name: "test-albatross",
                genesis: include!(concat!(
                    env!("OUT_DIR"),
                    "/genesis/test-albatross/genesis.rs"
                )),
            },
        );

        add(
            &mut m,
            NetworkInfo {
                network_id: NetworkId::UnitAlbatross,
                name: "unit-albatross",
                genesis: include!(concat!(
                    env!("OUT_DIR"),
                    "/genesis/unit-albatross/genesis.rs"
                )),
            },
        );

        m
    };
}
