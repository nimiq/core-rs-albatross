use beserial::{Deserialize, Serialize};
use crate::consensus::base::block::{Block, BlockHeader, BlockInterlink, BlockBody};
use crate::consensus::base::primitive::hash::Blake2bHash;
use crate::consensus::base::primitive::crypto::PublicKey;
use crate::consensus::base::primitive::Address;
use crate::network::address::PeerId;
use crate::network::address::peer_address::PeerAddressType;
use crate::network::address::peer_address::PeerAddress;
use crate::network::address::net_address::NetAddress;
use crate::utils::services::ServiceFlags;
use std::collections::HashMap;

#[derive(Serialize, Deserialize, Copy, Clone, Ord, PartialOrd, Eq, PartialEq, Debug, Hash)]
#[repr(u8)]
pub enum NetworkId {
    Test = 1,
    Dev = 2,
    Bounty = 3,
    Dummy = 4,
    Main = 42,
}

pub struct NetworkInfo {
    pub network_id: NetworkId,
    pub name: String,
    pub seed_peers: [PeerAddress; 20],
    pub genesis_block: Block,
    pub genesis_hash: Blake2bHash,
    pub accounts: String, // FIXME
}

fn create_seed_peer_addr(url: &str, port: u16, pubkey_hex: &str) -> PeerAddress {
    let mut public_key_bytes : [u8; PublicKey::SIZE] = [0; PublicKey::SIZE];
    public_key_bytes.clone_from_slice(&::hex::decode(pubkey_hex.to_string()).unwrap()[0..]);
    let public_key = PublicKey::from(&public_key_bytes);
    PeerAddress { ty: PeerAddressType::Wss(url.to_string(), port), services: ServiceFlags::FULL, timestamp: 0, net_address: NetAddress::Unspecified, public_key, distance: 0, signature: None, peer_id: PeerId::from(&public_key)}
}

lazy_static! {
    static ref NETWORK_MAP: HashMap<NetworkId, NetworkInfo> = {
        let mut m = HashMap::new();
        fn add(m: &mut HashMap<NetworkId, NetworkInfo>, info: NetworkInfo) { m.insert(info.network_id, info); }

        add(
            &mut m,
            NetworkInfo {
                network_id: NetworkId::Main,
                name: "main".into(),
                seed_peers: [
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
                genesis_block: Block {
                    header: BlockHeader {
                        version: 1,
                        prev_hash: [0u8; 32].into(),
                        interlink_hash: [0u8; 32].into(),
                        body_hash: "7cda9a7fdf06655905ae5dbd9c535451471b078fa6f3df0e287e5b0fb47a573a".into(),
                        accounts_hash: "1fefd44f1fa97185fda21e957545c97dc7643fa7e4efdd86e0aa4244d1e0bc5c".into(),
                        n_bits: 0x1f010000.into(),
                        height: 1,
                        timestamp: 1523727000,
                        nonce: 137689,
                    },
                    interlink: BlockInterlink::new(vec![], &[0u8; 32].into()),
                    body: Some(BlockBody {
                        miner: [0u8; Address::SIZE].into(),
                        extra_data: "love ai amor mohabbat hubun cinta lyubov bhalabasa amour kauna pi'ara liebe eshq upendo prema amore katresnan sarang anpu prema yeu".as_bytes().to_vec(),
                        transactions: vec![],
                        pruned_accounts: vec![],
                    }),
                },
                genesis_hash: "264aaf8a4f9828a76c550635da078eb466306a189fcc03710bee9f649c869d12".into(),
                accounts: String::new(),
            },
        );

        m
    };
}

pub fn get_network_info<'a>(network_id: NetworkId) -> Option<&'a NetworkInfo> { return NETWORK_MAP.get(&network_id); }
