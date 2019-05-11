#![allow(clippy::unreadable_literal)]

use std::collections::HashMap;

use hex::FromHex;

use hash::Blake2bHash;
use keys::Address;
use keys::PublicKey;
use block_albatross::{Block, MacroBlock, MacroHeader, MacroExtrinsics, PbftProof};
pub use primitives::networks::NetworkId;
use bls::bls12_381::{
    Signature as BlsSignature,
    PublicKey as BlsPublicKey,
};
use bls::Encoding;
use account::{Account,BasicAccount,StakingContract};
use primitives::coin::Coin;

use crate::address::net_address::NetAddress;
use crate::address::peer_address::PeerAddress;
use crate::address::peer_address::PeerAddressType;
use crate::address::PeerId;
use crate::address::seed_list::SeedList;
use crate::services::ServiceFlags;
use std::str::FromStr;


#[derive(Clone, Debug)]
pub struct NetworkInfo {
    pub network_id: NetworkId,
    pub name: String,
    pub seed_peers: Vec<PeerAddress>,
    pub seed_lists: Vec<SeedList>,
    pub genesis_block: Block,
    pub genesis_hash: Blake2bHash,
    pub genesis_accounts: Vec<(Address, Account)>,
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

fn block_seed(s: &str) -> BlsSignature {
    BlsSignature::from_slice(&hex::decode(s)
        .expect("Invalid hex representation"))
        .expect("Invalid BLS signature")
}

struct GenesisStakes(StakingContract);
impl GenesisStakes {
    pub fn new() -> Self {
        Self(StakingContract::default())
    }

    pub fn stake(mut self, staker_address: &str, reward_address: &str, balance: u64, public_key: &str) -> Self {
        self.0.stake(
            &Address::from_str(staker_address)
                .expect("Invalid staker address"),
            Coin::from_u64_unchecked(balance),
            BlsPublicKey::from_slice(&hex::decode(public_key)
                .expect("Invalid hex encoding"))
                .expect("Invalid BLS public key"),
            Some(Address::from_str(reward_address)
                .expect("Invalid reward address"))
        ).expect("Staking failed");
        self
    }

    pub fn finalize(self) -> (Address, Account) {
        (
            Address::from_user_friendly_address("").unwrap(), // TODO
            Account::Staking(self.0)
        )
    }
}

#[inline]
fn basic_account(address: &str, balance: u64) -> (Address, Account) {
    (
        Address::from_str(address)
            .expect("Invalid basic ccount address"),
        Account::Basic(BasicAccount { balance: Coin::from_u64_unchecked(balance) })
    )
}

lazy_static! {
    static ref NETWORK_MAP: HashMap<NetworkId, NetworkInfo> = {
        let mut m = HashMap::new();
        fn add(m: &mut HashMap<NetworkId, NetworkInfo>, info: NetworkInfo) { m.insert(info.network_id, info); }

        add(
            &mut m,
            NetworkInfo {
                network_id: NetworkId::AlbatrossTest,
                name: "albatross-test".into(),
                seed_peers: vec![
                    // TODO
                    //create_seed_peer_addr("seed1.nimiqtest.net", 8080, "175d5f01af8a5911c240a78df689a76eef782d793ca15d073bdc913edd07c74b"),
                ],
                seed_lists: vec![],

                // Seed: 'Albatross TestNet' (893aa07a38decc3919a99b94c0be9f00c378622fcfee9d90b27b7bb87d26cf3d)
                // Creation time: 2019-05-11T03:17:04.831169852+00:00
                genesis_block: Block::Macro(MacroBlock {
                    header: MacroHeader {
                        version: 1,
                        validators: vec![/*TODO*/],
                        block_number: 1,
                        view_number: 0,
                        parent_macro_hash: "0000000000000000000000000000000000000000000000000000000000000000".into(),
                        seed: block_seed("b85dce329205d9872ff281de313d3dfe5a6ed78334979fb4fcc26894944e9803ee3585265a99fb32aaacf8c5c9ef5952"),
                        parent_hash: "0000000000000000000000000000000000000000000000000000000000000000".into(),
                        state_root: "0000000000000000000000000000000000000000000000000000000000000000".into(),
                        extrinsics_root: "0000000000000000000000000000000000000000000000000000000000000000".into(),
                        timestamp: 1557544624831,
                    },
                    justification: None, // PbftProof::new(),
                    extrinsics: Some(MacroExtrinsics {
                        slot_allocation: vec![/*TODO*/],
                        slashing_amount: 100000
                    })
                }),
                genesis_hash: "a0390a392858a08ea4b5b96ad64d24f438e903597ac13469fbc8fbe592bc24aa".into(),
                genesis_accounts: vec![
                    GenesisStakes::new()
                        .stake("e302540287a45936af6b38139aa438be6056d000", "e302540287a45936af6b38139aa438be6056d000", 123, "85a92a104422b30cf669285eb2436414c5201f86f986f38b1e59806aefd39c0fea8f96e9d81bba942914234752288287136f26c34a836b7fe5cfba642d6ac579e0683648a5df8fac8f8d4ff9b0f38aaf053118d3021d21a3df10b67536ba5b9f")
                        .finalize(),
                    basic_account("e302540287a45936af6b38139aa438be6056d000", 123),
                ],

            }
        );

        m
    };
}

pub fn get_network_info<'a>(network_id: NetworkId) -> Option<&'a NetworkInfo> { NETWORK_MAP.get(&network_id) }
