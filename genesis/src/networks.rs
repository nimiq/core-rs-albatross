use std::collections::HashMap;
use std::env;
use std::path::Path;

use hex::FromHex;
use lazy_static::lazy_static;

use beserial::{Deserialize, Serialize};
use nimiq_account::Account;
use nimiq_account::AccountsList;
use nimiq_database::volatile::VolatileEnvironment;
use nimiq_genesis_builder::{GenesisBuilder, GenesisBuilderError, GenesisInfo};
use nimiq_hash::Blake2bHash;
use nimiq_keys::PublicKey;
use nimiq_peer_address::address::seed_list::SeedList;
use nimiq_peer_address::address::{NetAddress, PeerAddress, PeerAddressType, PeerId};
use nimiq_peer_address::services::ServiceFlags;
use nimiq_trie::key_nibbles::KeyNibbles;

pub use nimiq_primitives::networks::NetworkId;

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

    seed_peers: Vec<PeerAddress>,
    seed_lists: Vec<SeedList>,

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
    pub fn seed_peers(&self) -> &Vec<PeerAddress> {
        &self.seed_peers
    }

    #[inline]
    pub fn seed_lists(&self) -> &Vec<SeedList> {
        &self.seed_lists
    }

    #[inline]
    pub fn genesis_block<B: Deserialize>(&self) -> B {
        let block: B = Deserialize::deserialize_from_vec(self.genesis.block)
            .expect("Failed to deserialize genesis block.");
        block
    }

    #[inline]
    pub fn genesis_hash(&self) -> &Blake2bHash {
        &self.genesis.hash
    }

    #[inline]
    pub fn genesis_accounts(&self) -> Vec<(KeyNibbles, Account)> {
        let accounts: AccountsList = Deserialize::deserialize_from_vec(self.genesis.accounts)
            .expect("Failed to deserialize genesis accounts.");
        accounts.0
    }

    pub fn from_network_id(network_id: NetworkId) -> &'static Self {
        NETWORK_MAP
            .get(&network_id)
            .unwrap_or_else(|| panic!("No such network ID: {}", network_id))
    }
}

fn read_genesis_config(config: &Path) -> Result<GenesisData, GenesisBuilderError> {
    let env = VolatileEnvironment::new(10).expect("Could not open a volatile database");

    let GenesisInfo {
        block,
        hash,
        accounts,
    } = GenesisBuilder::new()
        .with_config_file(config)?
        .generate(env)?;

    let block = block.serialize_to_vec();
    let accounts = AccountsList(accounts).serialize_to_vec();

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

        let override_path = env::var_os("NIMIQ_OVERRIDE_DEVNET_CONFIG");
        let dev_genesis = if let Some(p) = override_path {
            read_genesis_config(Path::new(&p))
                .expect("failure reading provided NIMIQ_OVERRIDE_DEVNET_CONFIG")
        } else {
            include!(concat!(
                env!("OUT_DIR"),
                "/genesis/dev-albatross/genesis.rs"
            ))
        };

        add(
            &mut m,
            NetworkInfo {
                network_id: NetworkId::DevAlbatross,
                name: "dev-albatross",
                seed_peers: vec![create_seed_peer_addr_ws(
                    "seed1.validators.devnet.nimiq.dev",
                    8443,
                    "5af4c3f30998573e8d3476cd0e0543bf7adba576ef321342e41c2bccc246c377",
                )],
                seed_lists: vec![],
                genesis: dev_genesis,
            },
        );

        add(
            &mut m,
            NetworkInfo {
                network_id: NetworkId::UnitAlbatross,
                name: "unit-albatross",
                seed_peers: vec![],
                seed_lists: vec![],
                genesis: include!(concat!(
                    env!("OUT_DIR"),
                    "/genesis/unit-albatross/genesis.rs"
                )),
            },
        );

        m
    };
}

pub fn create_seed_peer_addr(url: &str, port: u16, pubkey_hex: &str) -> PeerAddress {
    let public_key = PublicKey::from_hex(pubkey_hex).unwrap();
    PeerAddress {
        ty: PeerAddressType::Wss(url.to_string(), port),
        services: ServiceFlags::FULL,
        timestamp: 0,
        net_address: NetAddress::Unspecified,
        public_key,
        distance: 0,
        signature: None,
        peer_id: PeerId::from(&public_key),
    }
}

pub fn create_seed_peer_addr_ws(url: &str, port: u16, pubkey_hex: &str) -> PeerAddress {
    let public_key = PublicKey::from_hex(pubkey_hex).unwrap();
    PeerAddress {
        ty: PeerAddressType::Ws(url.to_string(), port),
        services: ServiceFlags::FULL,
        timestamp: 0,
        net_address: NetAddress::Unspecified,
        public_key,
        distance: 0,
        signature: None,
        peer_id: PeerId::from(&public_key),
    }
}

pub fn create_seed_list(url_str: &str, pubkey_hex: &str) -> SeedList {
    let url = url::Url::parse(url_str).unwrap();
    let public_key = PublicKey::from_hex(pubkey_hex).unwrap();
    SeedList::new(url, Some(public_key))
}
