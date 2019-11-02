use blockchain_albatross::Blockchain;
use network_messages::AlbatrossMessageAdapter;

use crate::protocol::ConsensusProtocol;
use crate::consensus_agent::sync::FullSync;

pub struct AlbatrossConsensusProtocol {}
impl ConsensusProtocol for AlbatrossConsensusProtocol {
    type Blockchain = Blockchain;
    type MessageAdapter = AlbatrossMessageAdapter;
    type SyncProtocol = FullSync<Self::Blockchain>;
}
