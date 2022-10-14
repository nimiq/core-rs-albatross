use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use parking_lot::RwLock;

use nimiq_blockchain::Blockchain;
use nimiq_mempool::mempool::{ControlTransactionTopic, TransactionTopic};
use nimiq_network_interface::network::Network;
use nimiq_primitives::account::AccountType;
use nimiq_transaction::Transaction;
use tokio::sync::broadcast::Sender as BroadcastSender;
use tokio_stream::wrappers::BroadcastStream;

use crate::ConsensusEvent;

pub struct ConsensusProxy<N: Network> {
    pub blockchain: Arc<RwLock<Blockchain>>,
    pub network: Arc<N>,
    pub(crate) established_flag: Arc<AtomicBool>,
    pub(crate) events: BroadcastSender<ConsensusEvent>,
}

impl<N: Network> Clone for ConsensusProxy<N> {
    fn clone(&self) -> Self {
        Self {
            blockchain: Arc::clone(&self.blockchain),
            network: Arc::clone(&self.network),
            established_flag: Arc::clone(&self.established_flag),
            events: self.events.clone(),
        }
    }
}

impl<N: Network> ConsensusProxy<N> {
    pub async fn send_transaction(&self, tx: Transaction) -> Result<(), N::Error> {
        if tx.sender_type == AccountType::Staking || tx.recipient_type == AccountType::Staking {
            return self.network.publish::<ControlTransactionTopic>(tx).await;
        }
        self.network.publish::<TransactionTopic>(tx).await
    }

    pub fn is_established(&self) -> bool {
        self.established_flag.load(Ordering::Acquire)
    }

    pub fn subscribe_events(&self) -> BroadcastStream<ConsensusEvent> {
        BroadcastStream::new(self.events.subscribe())
    }
}
