use std::sync::Arc;

use futures::StreamExt;

use crate::messages::handlers::Handle;
use crate::messages::{
    RequestBatchSet, RequestBlock, RequestBlockHashes, RequestHistoryChunk, RequestMissingBlocks,
};
use crate::Consensus;

use blockchain_albatross::Blockchain;
use network_interface::prelude::{Network, Peer};

impl<N: Network> Consensus<N> {
    pub(super) fn init_network_requests(network: &Arc<N>, blockchain: &Arc<Blockchain>) {
        let blockchain_outer = blockchain;
        let blockchain = Arc::clone(blockchain_outer);
        let mut stream = network.receive_from_all::<RequestBlockHashes>();
        tokio::spawn(async move {
            while let Some((msg, peer)) = stream.next().await {
                trace!(
                    "[REQUEST_BLOCK_HASHES] {} block locators received from {:?}",
                    msg.locators.len(),
                    peer.id()
                );

                if let Some(response) = msg.handle(&blockchain) {
                    // We do not care about the result.
                    let _ = peer.send(&response).await;
                }
            }
        });

        let blockchain = Arc::clone(blockchain_outer);
        let mut stream = network.receive_from_all::<RequestBatchSet>();
        tokio::spawn(async move {
            while let Some((msg, peer)) = stream.next().await {
                trace!(
                    "[REQUEST_EPOCH] for block {:?} received from {:?}",
                    msg.hash,
                    peer.id()
                );

                if let Some(response) = msg.handle(&blockchain) {
                    // We do not care about the result.
                    let _ = peer.send(&response).await;
                }
            }
        });

        let blockchain = Arc::clone(blockchain_outer);
        let mut stream = network.receive_from_all::<RequestHistoryChunk>();
        tokio::spawn(async move {
            while let Some((msg, peer)) = stream.next().await {
                trace!(
                    "[REQUEST_HISTORY_CHUNK] for epoch {}, chunk {} received from {:?}",
                    msg.epoch_number,
                    msg.chunk_index,
                    peer.id()
                );

                if let Some(response) = msg.handle(&blockchain) {
                    // We do not care about the result.
                    let _ = peer.send(&response).await;
                }
            }
        });

        let blockchain = Arc::clone(blockchain_outer);
        let mut stream = network.receive_from_all::<RequestBlock>();
        tokio::spawn(async move {
            while let Some((msg, peer)) = stream.next().await {
                trace!(
                    "[REQUEST_BLOCK] for block hash {} received from {:?}",
                    msg.hash,
                    peer.id()
                );

                if let Some(response) = msg.handle(&blockchain) {
                    // We do not care about the result.
                    let _ = peer.send(&response).await;
                }
            }
        });

        let blockchain = Arc::clone(blockchain_outer);
        let mut stream = network.receive_from_all::<RequestMissingBlocks>();
        tokio::spawn(async move {
            while let Some((msg, peer)) = stream.next().await {
                trace!(
                    "[REQUEST_MISSING_BLOCKS] for target_hash {} received from {:?}",
                    msg.target_hash,
                    peer.id()
                );

                if let Some(response) = msg.handle(&blockchain) {
                    // We do not care about the result.
                    let _ = peer.send(&response).await;
                }
            }
        });
    }
}
