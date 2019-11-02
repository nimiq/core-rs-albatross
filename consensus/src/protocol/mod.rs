use blockchain_base::AbstractBlockchain;
use network_messages::MessageAdapter;

use crate::consensus_agent::sync::SyncProtocol;

pub mod albatross;
pub mod nimiq;

pub trait ConsensusProtocol {
    type Blockchain: AbstractBlockchain + 'static;
    type MessageAdapter: MessageAdapter<<Self::Blockchain as AbstractBlockchain>::Block> + 'static;
    type SyncProtocol: SyncProtocol<Self::Blockchain> + 'static;
}
