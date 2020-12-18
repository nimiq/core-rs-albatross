use std::sync::Arc;

use async_trait::async_trait;

use network_interface::network::Network;

use crate::consensus::Consensus;
use crate::error::SyncError;

pub use self::quick::*;

pub mod block_queue;
pub mod history;
mod quick;
pub mod request_component;
mod sync_queue;

#[async_trait]
pub trait SyncProtocol<N: Network>: Send + Sync + 'static {
    async fn perform_sync(&self, consensus: Arc<Consensus<N>>) -> Result<(), SyncError>;

    fn is_established(&self) -> bool;
}
