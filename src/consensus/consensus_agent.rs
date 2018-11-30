use std::collections::HashSet;
use std::sync::Arc;
use crate::consensus::base::blockchain::Blockchain;
use crate::consensus::base::mempool::Mempool;
use crate::consensus::base::primitive::hash::Blake2bHash;
use crate::network::Peer;
use crate::network::message::InvVector;
use crate::consensus::inventory::InventoryManager;
use parking_lot::RwLock;
use crate::consensus::inventory::InventoryAgent;
use crate::network::message::GetBlocksMessage;
use crate::network::message::GetBlocksDirection;

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ConsensusAgentEvent {

}

pub struct ConsensusAgent {
    blockchain: Arc<Blockchain<'static>>,
    mempool: Arc<Mempool<'static>>,
    peer: Arc<Peer>,

    inv_agent: Arc<RwLock<InventoryAgent>>,

    /// Flag indicating that we are currently syncing our blockchain with the peer's.
    syncing: bool,

    /// Flag indicating that we have synced our blockchain with the peer's.
    synced: bool,

    /// The hash of the block that we want to learn to consider the sync complete.
    sync_target: Blake2bHash,

    /// The hash of the last fork block the peer has sent us.
    fork_head: Option<Blake2bHash>,

    /// The number of blocks that extended our blockchain since the last requestBlocks().
    num_blocks_extending: usize,

    /// The number of blocks that forked our blockchain since the last requestBlocks().
    num_blocks_forking: usize,

    /// The number of failed blockchain sync attempts.
    failed_syncs: usize,

}

impl ConsensusAgent {
    pub fn new(blockchain: Arc<Blockchain<'static>>, mempool: Arc<Mempool<'static>>, inv_mgr: Arc<RwLock<InventoryManager>>, peer: Peer) -> Arc<RwLock<Self>> {
        let sync_target = peer.head_hash.clone();
        let peer_arc = Arc::new(peer);
        let inv_agent = InventoryAgent::new(blockchain.clone(), mempool.clone(), inv_mgr,peer_arc.clone());
        let this = Arc::new(RwLock::new(ConsensusAgent {
            blockchain,
            mempool,
            peer: peer_arc.clone(),
            inv_agent,
            syncing: false,
            synced: false,
            sync_target,
            fork_head: None,
            num_blocks_extending: 0,
            num_blocks_forking: 0,
            failed_syncs: 0
        }));
        this
    }

    pub fn sync(&mut self) {
        self.syncing = true;
        self.inv_agent.write().bypass_mgr(true);

        self.peer.channel.send(GetBlocksMessage::new(
            vec![self.blockchain.head_hash()],
            500,
            GetBlocksDirection::Forward,
        )).unwrap();
    }

    fn sync_finished(&mut self) {

    }
}
