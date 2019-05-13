use blockchain::Blockchain;
use network_messages::NimiqMessageAdapter;

use crate::protocol::ConsensusProtocol;

pub struct NimiqConsensusProtocol {}
impl ConsensusProtocol for NimiqConsensusProtocol {
    type Blockchain = Blockchain<'static>;
    type MessageAdapter = NimiqMessageAdapter;
}
