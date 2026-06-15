use std::sync::{atomic::AtomicU32, Arc};

use nimiq_block::Block;
use nimiq_blockchain::{Blockchain, BlockchainConfig};
use nimiq_blockchain_proxy::BlockchainProxy;
use nimiq_consensus::{sync::syncer_proxy::SyncerProxy, BlsCache, Consensus};
use nimiq_database::mdbx::MdbxDatabase;
use nimiq_genesis_builder::GenesisInfo;
use nimiq_network_interface::network::Network as NetworkInterface;
use nimiq_network_mock::MockHub;
use nimiq_primitives::{networks::NetworkId, trie::TrieItem};
use nimiq_utils::{spawn, time::OffsetTime};
use parking_lot::{Mutex, RwLock};

use crate::test_network::TestNetwork;

pub struct Node<N: NetworkInterface + TestNetwork> {
    pub network: Arc<N>,
    pub blockchain: Arc<RwLock<Blockchain>>,
    pub consensus: Option<Consensus<N>>,
    pub environment: MdbxDatabase,
}

impl<N: NetworkInterface + TestNetwork> Node<N> {
    pub async fn history_with_genesis_info(
        peer_id: u64,
        genesis_info: GenesisInfo,
        hub: &mut Option<MockHub>,
    ) -> Self {
        Self::new_history(
            peer_id,
            genesis_info.block,
            genesis_info.accounts.expect("history nodes need accounts"),
            hub,
        )
        .await
    }

    pub async fn new_history(
        peer_id: u64,
        block: Block,
        accounts: Vec<TrieItem>,
        hub: &mut Option<MockHub>,
    ) -> Self {
        let block_hash = block.hash();
        let env = MdbxDatabase::new_volatile(Default::default()).unwrap();
        let clock = Arc::new(OffsetTime::new());
        let blockchain = Arc::new(RwLock::new(
            Blockchain::with_genesis(
                env.clone(),
                BlockchainConfig::default(),
                Arc::clone(&clock),
                NetworkId::UnitAlbatross,
                block,
                Some(accounts),
            )
            .unwrap(),
        ));

        let network = N::build_network(peer_id, block_hash, hub).await;

        let blockchain_proxy = BlockchainProxy::Full(Arc::clone(&blockchain));
        let syncer = SyncerProxy::new_history(
            blockchain_proxy.clone(),
            Arc::clone(&network),
            Arc::new(Mutex::new(BlsCache::new_test())),
            network.subscribe_events(),
        )
        .await;
        let consensus = Consensus::<N>::new(
            blockchain_proxy,
            Arc::clone(&network),
            syncer,
            1,
            Arc::new(AtomicU32::new(0)),
        );

        Node {
            network,
            blockchain,
            consensus: Some(consensus),
            environment: env,
        }
    }

    pub fn consume(&mut self) {
        if let Some(consensus) = self.consensus.take() {
            spawn(consensus);
        }
    }
}
