use blockchain_albatross::Blockchain;
use network_messages::AlbatrossMessageAdapter;

use crate::consensus_agent::sync::FullSync;
use crate::protocol::ConsensusProtocol;

pub struct AlbatrossConsensusProtocol {}
impl ConsensusProtocol for AlbatrossConsensusProtocol {
    type Blockchain = Blockchain;
    type MessageAdapter = AlbatrossMessageAdapter;
    type SyncProtocol = FullSync<Self::Blockchain>;
}
