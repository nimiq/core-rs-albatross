use blockchain_base::AbstractBlockchain;
use network_messages::MessageAdapter;

use crate::consensus_agent::sync::SyncProtocol;

pub mod albatross;
pub mod nimiq;

pub trait ConsensusProtocol {
    type Blockchain: AbstractBlockchain<'static> + 'static;
    type MessageAdapter: MessageAdapter<<Self::Blockchain as AbstractBlockchain<'static>>::Block> + 'static;
    type SyncProtocol: SyncProtocol<'static, Self::Blockchain>;
}
