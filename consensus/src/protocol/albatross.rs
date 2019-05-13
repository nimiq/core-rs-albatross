use blockchain_albatross::Blockchain;
use network_messages::AlbatrossMessageAdapter;

use crate::protocol::ConsensusProtocol;

pub struct AlbatrossConsensusProtocol {}
impl ConsensusProtocol for AlbatrossConsensusProtocol {
    type Blockchain = Blockchain<'static>;
    type MessageAdapter = AlbatrossMessageAdapter;
}
