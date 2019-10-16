use blockchain::Blockchain;
use network_messages::NimiqMessageAdapter;

use crate::consensus_agent::sync::FullSync;
use crate::protocol::ConsensusProtocol;

pub struct NimiqConsensusProtocol {}
impl ConsensusProtocol for NimiqConsensusProtocol {
    type Blockchain = Blockchain<'static>;
    type MessageAdapter = NimiqMessageAdapter;
    type SyncProtocol = FullSync<'static, Self::Blockchain>;
}
