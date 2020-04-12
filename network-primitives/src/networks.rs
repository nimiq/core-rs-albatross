pub use primitives::networks::NetworkId;
use crate::address::net_address::NetAddress;
use crate::address::peer_address::PeerAddress;
use crate::address::peer_address::PeerAddressType;
use crate::address::PeerId;
use crate::services::ServiceFlags;
use crate::address::seed_list::SeedList;
use beserial::Deserialize;
use hash::Blake2bHash;
use keys::Address;
use account::Account;
use std::collections::HashMap;
use keys::PublicKey;
use hex::FromHex;
use account::AccountsList;
use lazy_static::lazy_static;


#[derive(Clone, Debug)]
struct GenesisData {
    block: &'static [u8],
    hash: Blake2bHash,
    accounts: &'static [u8],
    validator_registry: Option<Address>,
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
        let block: B = Deserialize::deserialize_from_vec(&self.genesis.block.to_vec())
            .expect("Failed to deserialize genesis block.");
        block
    }

    #[inline]
    pub fn genesis_hash(&self) -> &Blake2bHash {
        &self.genesis.hash
    }

    #[inline]
    pub fn genesis_accounts(&self) -> Vec<(Address, Account)> {
        let accounts: AccountsList = Deserialize::deserialize_from_vec(&self.genesis.accounts.to_vec())
            .expect("Failed to deserialize genesis accounts.");
        accounts.0
    }

    #[inline]
    pub fn validator_registry_address(&self) -> Option<&Address> {
        self.genesis.validator_registry.as_ref()
    }

    pub fn from_network_id(network_id: NetworkId) -> &'static Self {
        NETWORK_MAP.get(&network_id)
            .unwrap_or_else(|| panic!("No such network ID: {}", network_id))
    }
}



lazy_static! {
    static ref NETWORK_MAP: HashMap<NetworkId, NetworkInfo> = {
        let mut m = HashMap::new();
        fn add(m: &mut HashMap <NetworkId, NetworkInfo>, info: NetworkInfo) {
            m.insert(info.network_id, info);
        }

        add(&mut m, NetworkInfo {
            network_id: NetworkId::Main,
            name: "main",
            seed_peers: vec![
                create_seed_peer_addr("seed-1.nimiq.com", 8443, "b70d0c3e6cdf95485cac0688b086597a5139bc4237173023c83411331ef90507"),
                create_seed_peer_addr("seed-2.nimiq.com", 8443, "8580275aef426981a04ee5ea948ca3c95944ef1597ad78db9839f810d6c5b461"),
                create_seed_peer_addr("seed-3.nimiq.com", 8443, "136bdec59f4d37f25ac8393bef193ff2e31c9c0a024b3edbf77fc1cb84e67a15"),
                create_seed_peer_addr("seed-4.nimiq-network.com", 8443, "aacf606335cdd92d0dd06f27faa3b66d9bac0b247cd57ade413121196b72cd73"),
                create_seed_peer_addr("seed-5.nimiq-network.com", 8443, "110a81a033c75976643d4b8f34419f4913b306a6fc9d530b8207ddbd5527eff6"),
                create_seed_peer_addr("seed-6.nimiq-network.com", 8443, "26c1a4727cda6579639bdcbaecb1f6b97be3ac0e282b43bdd1a2df2858b3c23b"),
                create_seed_peer_addr("seed-7.nimiq.network", 8443, "82fcebdb4e2a7212186d1976d7f685cc86cdf58beffe1723d5c3ea5be00c73e1"),
                create_seed_peer_addr("seed-8.nimiq.network", 8443, "b7ac8cc1a820761df4e8a42f4e30c870e81065c4e29f994ebb5bdceb48904e7b"),
                create_seed_peer_addr("seed-9.nimiq.network", 8443, "4429bf25c8d296c0f1786647d8f7d4bac40a37c67caf028818a65a9cc7865a48"),
                create_seed_peer_addr("seed-10.nimiq.network", 8443, "e8e99fb8633d660d4f2d48edb6cc294681b57648b6ec6b28af8f85b2d5ec4e68"),
                create_seed_peer_addr("seed-11.nimiq.network", 8443, "a76f0edabacfe701750036bad473ff92fa0e68ef655ab93135f0572af6e5baf8"),
                create_seed_peer_addr("seed-12.nimiq.network", 8443, "dca57704191306ac1315e051b6dfef6c174fb2af011a52a3d922fbfaec2be41a"),
                create_seed_peer_addr("seed-13.nimiq-network.com", 8443, "30993f92f148da125a6f8bc191b3e746fab39e109220daa0966bf6432e909f3f"),
                create_seed_peer_addr("seed-14.nimiq-network.com", 8443, "6e7f904fabfadb194d6c74b16534bacb69892d80909cf959e47d3c8f5f330ad2"),
                create_seed_peer_addr("seed-15.nimiq-network.com", 8443, "7cb662a686144c17ae4153fbf7ce359f7e9da39dc072eb11092531f9104fbdf6"),
                create_seed_peer_addr("seed-16.nimiq.com", 8443, "0dfd11939947101197e3c3768a086e65ef1e893e71bfcf4bd5ed222957825212"),
                create_seed_peer_addr("seed-17.nimiq.com", 8443, "c7120f4f88b70a38daa9783e30e89c1c55c3d80d0babed44b4e2ddd09052664a"),
                create_seed_peer_addr("seed-18.nimiq.com", 8443, "c15a2d824a52837fa7165dc232592be35116661e7f28605187ab273dd7233711"),
                create_seed_peer_addr("seed-19.nimiq.com", 8443, "98a24d4b05158314b36e0bd6ce3b42ac5ac061f4bb9664d783eb930caa9315b6"),
                create_seed_peer_addr("seed-20.nimiq.com", 8443, "1fc33f93273d94dd2cf7470274c27ecb1261ec983e43bdbb281803c0a09e68d5")
            ],
            seed_lists: vec![
                create_seed_list("https://nimiq.community/seeds.txt", "8b4ae04557f490102036ce3e570b39058c92fc5669083fb9bbb6effc91dc3c71")
            ],
            genesis: include!(concat!(env!("OUT_DIR"), "/genesis/main-powchain/genesis.rs")),
        });

        add(&mut m, NetworkInfo {
            network_id: NetworkId::Test,
            name: "test",
            seed_peers: vec![
                create_seed_peer_addr("seed1.nimiq-testnet.com", 8080, "175d5f01af8a5911c240a78df689a76eef782d793ca15d073bdc913edd07c74b"),
                create_seed_peer_addr("seed2.nimiq-testnet.com", 8080, "2c950d2afad1aa7ad12f01a56527f709b7687b1b00c94da6e0bd8ae4d263d47c"),
                create_seed_peer_addr("seed3.nimiq-testnet.com", 8080, "03feec9d5316a7b5ebb69c4e709547a28afe8e9ef91ee568df489d29e9845bb8"),
                create_seed_peer_addr("seed4.nimiq-testnet.com", 8080, "943d5669226d3716a830371d99143af98bbaf84c630db24bdd67e55ccb7a9011")
            ],
            seed_lists: vec![],
            genesis: include!(concat!(env!("OUT_DIR"), "/genesis/test-powchain/genesis.rs")),
        });

        add(&mut m, NetworkInfo {
            network_id: NetworkId::Dev,
            name: "dev",
            seed_peers: vec![
                create_seed_peer_addr_ws("dev.nimiq-network.com", 8080, "e65e39616662f2c16d62dc08915e5a1d104619db8c2b9cf9b389f96c8dce9837")
            ],
            seed_lists: vec![],
            genesis: include!(concat!(env!("OUT_DIR"), "/genesis/test-powchain/genesis.rs")),
        });

        add(&mut m, NetworkInfo {
            network_id: NetworkId::DevAlbatross,
            name: "dev-albatross",
            seed_peers: vec![
                //create_seed_peer_addr_ws("albatross.nimiq.dev", 8444, "5af4c3f30998573e8d3476cd0e0543bf7adba576ef321342e41c2bccc246c377"),
            ],
            seed_lists: vec![],
            genesis: include!(concat!(env!("OUT_DIR"), "/genesis/dev-albatross/genesis.rs")),
        });

        add(&mut m, NetworkInfo {
            network_id: NetworkId::UnitAlbatross,
            name: "unit-albatross",
            seed_peers: vec![],
            seed_lists: vec![],
            genesis: include!(concat!(env!("OUT_DIR"), "/genesis/unit-albatross/genesis.rs")),
        });

        m
    };
}



fn create_seed_peer_addr(url: &str, port: u16, pubkey_hex: &str) -> PeerAddress {
    let public_key = PublicKey::from_hex(pubkey_hex).unwrap();
    PeerAddress { ty: PeerAddressType::Wss(url.to_string(), port), services: ServiceFlags::FULL, timestamp: 0, net_address: NetAddress::Unspecified, public_key, distance: 0, signature: None, peer_id: PeerId::from(&public_key)}
}

fn create_seed_peer_addr_ws(url: &str, port: u16, pubkey_hex: &str) -> PeerAddress {
    let public_key = PublicKey::from_hex(pubkey_hex).unwrap();
    PeerAddress { ty: PeerAddressType::Ws(url.to_string(), port), services: ServiceFlags::FULL, timestamp: 0, net_address: NetAddress::Unspecified, public_key, distance: 0, signature: None, peer_id: PeerId::from(&public_key)}
}

fn create_seed_list(url_str: &str, pubkey_hex: &str) -> SeedList {
    let url = url::Url::parse(url_str).unwrap();
    let public_key = PublicKey::from_hex(pubkey_hex).unwrap();
    SeedList::new(url, Some(public_key))
}
