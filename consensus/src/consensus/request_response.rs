use std::sync::Arc;

use futures::StreamExt;

use crate::messages::handlers::Handle;
use crate::messages::{
    RequestBatchSet, RequestBlock, RequestBlockHashes, RequestHead, RequestHistoryChunk,
    RequestMissingBlocks,
};
use crate::Consensus;

use blockchain::Blockchain;
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

                // Try to send the response, logging to debug if it fails
                if let Err(err) = peer.send(&msg.handle(&blockchain)).await {
                    log::debug!("Failed to send RequestBlockHashes Response: {:?}", err);
                };
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

                // Try to send the response, logging to debug if it fails
                if let Err(err) = peer.send(&msg.handle(&blockchain)).await {
                    log::debug!("Failed to send RequestEpoch Response: {:?}", err);
                };
            }
        });

        let blockchain = Arc::clone(blockchain_outer);
        let mut stream = network.receive_from_all::<RequestHistoryChunk>();
        tokio::spawn(async move {
            while let Some((msg, peer)) = stream.next().await {
                trace!(
                    "[REQUEST_HISTORY_CHUNK] for epoch {}, chunk {} with respect to block_number: {} received from {:?}",
                    msg.epoch_number,
                    msg.chunk_index,
                    msg.block_number,
                    peer.id()
                );

                // Try to send the response, logging to debug if it fails
                if let Err(err) = peer.send(&msg.handle(&blockchain)).await {
                    log::debug!("Failed to send RequestHistoryChunks Response: {:?}", err);
                };
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

                // Try to send the response, logging to debug if it fails
                if let Err(err) = peer.send(&msg.handle(&blockchain)).await {
                    log::debug!("Failed to send RequestBlocks Response: {:?}", err);
                };
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

                // Try to send the response, logging to debug if it fails
                if let Err(err) = peer.send(&msg.handle(&blockchain)).await {
                    log::debug!("Failed to send RequestMissingBlocks Response: {:?}", err);
                };
            }
        });

        let blockchain = Arc::clone(blockchain_outer);
        let mut stream = network.receive_from_all::<RequestHead>();
        tokio::spawn(async move {
            while let Some((msg, peer)) = stream.next().await {
                trace!("[REQUEST_HEAD] received from {:?}", peer.id());

                // Try to send the response, logging to debug if it fails
                if let Err(err) = peer.send(&msg.handle(&blockchain)).await {
                    log::debug!("Failed to send RequestHead Response: {:?}", err);
                };
            }
        });
    }
}
