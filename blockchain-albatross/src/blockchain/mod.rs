use std::iter::{Chain, Flatten, Map};
use std::vec::IntoIter;

use block::{Block, BlockError, ForkProof};
pub use blockchain::Blockchain;
#[cfg(feature = "metrics")]
use blockchain_base::chain_metrics::BlockchainMetrics;
pub use blockchain_state::BlockchainState;
use transaction::Transaction as BlockchainTransaction;

mod blockchain;
mod blockchain_state;

pub type PushResult = blockchain_base::PushResult;
pub type PushError = blockchain_base::PushError<BlockError>;
pub type BlockchainEvent = blockchain_base::BlockchainEvent<Block>;
pub type TransactionsIterator = Chain<
    IntoIter<BlockchainTransaction>,
    Flatten<Map<IntoIter<Block>, fn(Block) -> Vec<BlockchainTransaction>>>,
>;

#[derive(Debug, Eq, PartialEq)]
enum ChainOrdering {
    Extend,
    Better,
    Inferior,
    Unknown,
}

pub enum ForkEvent {
    Detected(ForkProof),
}

pub enum OptionalCheck<T> {
    Some(T),
    None,
    Skip,
}

impl<T> OptionalCheck<T> {
    pub fn is_some(&self) -> bool {
        if let OptionalCheck::Some(_) = self {
            true
        } else {
            false
        }
    }

    pub fn is_none(&self) -> bool {
        if let OptionalCheck::None = self {
            true
        } else {
            false
        }
    }

    pub fn is_skip(&self) -> bool {
        if let OptionalCheck::Skip = self {
            true
        } else {
            false
        }
    }
}

impl<T> From<Option<T>> for OptionalCheck<T> {
    fn from(opt: Option<T>) -> Self {
        match opt {
            Some(t) => OptionalCheck::Some(t),
            None => OptionalCheck::None,
        }
    }
}
