//! Synchronization logic for different types of sync in the consensus layer.

use std::sync::Arc;

use nimiq_block::Block;
use nimiq_hash::Blake2bHash;
use nimiq_network_interface::{network::Network, request::RequestError};

use crate::messages::{BlockError, RequestBlock};

#[cfg(feature = "full")]
pub mod history;
pub mod light;
pub mod live;
pub mod peer_list;
pub mod pico;
pub mod sync_interface;
mod sync_queue;
pub mod syncer;
pub mod syncer_proxy;

/// Requests a block by hash from the given peer, optionally including the body.
///
/// Verifies that the returned block matches the requested hash, returning
/// `BlockError::ResponseHashMismatch` otherwise, so a peer cannot answer a
/// request with a different block than the one we asked for.
pub(crate) async fn request_block<TNetwork: Network>(
    network: Arc<TNetwork>,
    peer_id: TNetwork::PeerId,
    hash: Blake2bHash,
    include_body: bool,
) -> Result<Result<Block, BlockError>, RequestError> {
    network
        .request::<RequestBlock>(
            RequestBlock {
                hash: hash.clone(),
                include_body,
            },
            peer_id,
        )
        .await
        .map(|block_res| {
            Ok(block_res.and_then(|mut block| {
                if block.hash_cached() != hash {
                    return Err(BlockError::ResponseHashMismatch);
                }
                Ok(block)
            }))
        })?
}
