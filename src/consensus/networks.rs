use beserial::{Deserialize, Serialize};
use consensus::base::block::{Block, BlockHeader, BlockInterlink};
use hex;
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
    pub seed_peers: Vec<String>, // FIXME
    pub genesis_block: Block,
    pub accounts: String, // FIXME
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
                seed_peers: Vec::new(),
                genesis_block: Block {
                    header: BlockHeader {
                        version: 1,
                        prev_hash: [0u8; 32].into(),
                        interlink_hash: [0u8; 32].into(),
                        body_hash: hex::decode("7cda9a7fdf06655905ae5dbd9c535451471b078fa6f3df0e287e5b0fb47a573a").unwrap()[..].into(),
                        accounts_hash: hex::decode("1fefd44f1fa97185fda21e957545c97dc7643fa7e4efdd86e0aa4244d1e0bc5c").unwrap()[..].into(),
                        n_bits: 0x1f010000.into(),
                        height: 1,
                        timestamp: 1523727000,
                        nonce: 137689,
                    },
                    interlink: BlockInterlink::new(vec![], &[0u8; 32].into()),
                    body: None,
                },
                accounts: String::new(),
            },
        );

        m
    };
}

pub fn get_network_info<'a>(network_id: NetworkId) -> Option<&'a NetworkInfo> { return NETWORK_MAP.get(&network_id); }
