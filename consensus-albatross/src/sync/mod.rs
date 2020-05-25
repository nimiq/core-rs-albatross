use async_trait::async_trait;
use network_interface::network::Network;
use std::sync::Arc;

pub use self::quick::*;
use crate::consensus::Consensus;
use crate::error::SyncError;

mod quick;
mod sync_queue;

#[async_trait]
pub trait SyncProtocol<N: Network>: Send + Sync + 'static {
    async fn perform_sync(&self, consensus: Arc<Consensus<N>>) -> Result<(), SyncError>;

    fn is_established(&self) -> bool;
}
