use std::path::PathBuf;
use std::sync::Arc;

use nimiq_bls::cache::PublicKeyCache;
use parking_lot::{Mutex, RwLock};

use nimiq_account::Account;
use nimiq_block::Block;
use nimiq_blockchain::{Blockchain, BlockchainConfig};
use nimiq_blockchain_proxy::BlockchainProxy;
use nimiq_consensus::{sync::syncer_proxy::SyncerProxy, Consensus as AbstractConsensus};
use nimiq_database::volatile::VolatileEnvironment;
use nimiq_genesis_builder::GenesisInfo;
use nimiq_network_interface::network::Network as NetworkInterface;
use nimiq_network_mock::MockHub;
use nimiq_primitives::networks::NetworkId;
use nimiq_trie::key_nibbles::KeyNibbles;
use nimiq_utils::time::OffsetTime;
use nimiq_zkp_component::ZKPComponent;

use crate::test_network::TestNetwork;
use crate::zkp_test_data::{zkp_test_exe, KEYS_PATH};

pub const TESTING_BLS_CACHE_MAX_CAPACITY: usize = 100;

pub struct Node<N: NetworkInterface + TestNetwork> {
    pub network: Arc<N>,
    pub blockchain: Arc<RwLock<Blockchain>>,
    pub consensus: Option<AbstractConsensus<N>>,
}

impl<N: NetworkInterface + TestNetwork> Node<N> {
    pub async fn history_with_genesis_info(
        peer_id: u64,
        genesis_info: GenesisInfo,
        hub: &mut Option<MockHub>,
        is_prover_active: bool,
    ) -> Self {
        Self::new_history(
            peer_id,
            genesis_info.block,
            genesis_info.accounts,
            hub,
            is_prover_active,
        )
        .await
    }

    pub async fn new_history(
        peer_id: u64,
        block: Block,
        accounts: Vec<(KeyNibbles, Account)>,
        hub: &mut Option<MockHub>,
        is_prover_active: bool,
    ) -> Self {
        let block_hash = block.hash();
        let env = VolatileEnvironment::new(14).unwrap();
        let clock = Arc::new(OffsetTime::new());
        let blockchain = Arc::new(RwLock::new(
            Blockchain::with_genesis(
                env.clone(),
                BlockchainConfig::default(),
                Arc::clone(&clock),
                NetworkId::UnitAlbatross,
                block,
                accounts,
            )
            .unwrap(),
        ));

        let network = N::build_network(peer_id, block_hash, hub).await;
        let zkp_proxy = ZKPComponent::new(
            BlockchainProxy::from(&blockchain),
            Arc::clone(&network),
            is_prover_active,
            Some(zkp_test_exe()),
            env.clone(),
            PathBuf::from(KEYS_PATH),
        )
        .await;

        let blockchain_proxy = BlockchainProxy::Full(Arc::clone(&blockchain));
        let syncer = SyncerProxy::new_history(
            blockchain_proxy.clone(),
            Arc::clone(&network),
            Arc::new(Mutex::new(PublicKeyCache::new(
                TESTING_BLS_CACHE_MAX_CAPACITY,
            ))),
            network.subscribe_events(),
        )
        .await;
        let consensus = AbstractConsensus::<N>::new(
            env,
            blockchain_proxy,
            Arc::clone(&network),
            syncer,
            1,
            zkp_proxy.proxy(),
        );

        Node {
            network,
            blockchain,
            consensus: Some(consensus),
        }
    }

    pub fn consume(&mut self) {
        if let Some(consensus) = self.consensus.take() {
            tokio::spawn(consensus);
        }
    }
}
