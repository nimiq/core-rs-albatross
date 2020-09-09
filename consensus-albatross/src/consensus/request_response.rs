use crate::messages::{
    BlockHashes, Epoch, RequestBlockHashes, RequestBlockHashesFilter, RequestEpoch,
    RequestResponseMessage,
};
use crate::Consensus;
use block_albatross::Block;
use blockchain_albatross::{Blockchain, Direction};
use futures::StreamExt;
use network_interface::prelude::{Network, Peer, ResponseMessage};
use nimiq_genesis::NetworkInfo;
use primitives::policy;
use std::sync::Arc;

impl<N: Network> Consensus<N> {
    pub(super) fn init_network_requests(network: &Arc<N>, blockchain: &Arc<Blockchain>) {
        let blockchain_outer = blockchain;
        let blockchain = Arc::clone(blockchain_outer);
        let mut stream = network.receive_from_all::<RequestResponseMessage<RequestBlockHashes>>();
        tokio::spawn(async move {
            while let Some((msg, peer)) = stream.next().await {
                trace!(
                    "[REQUEST_BLOCK_HASHES] {} block locators received from {:?}",
                    msg.locators.len(),
                    peer.id()
                );

                // A peer has requested blocks. Check all requested block locator hashes
                // in the given order and pick the first hash that is found on our main
                // chain, ignore the rest. If none of the requested hashes is found,
                // pick the genesis block hash. Send the main chain starting from the
                // picked hash back to the peer.
                let network_info = NetworkInfo::from_network_id(blockchain.network_id);
                let mut start_block_hash = network_info.genesis_hash().clone();
                for locator in msg.locators.iter() {
                    if blockchain
                        .chain_store
                        .get_block(locator, false, None)
                        .is_some()
                    {
                        // We found a block, ignore remaining block locator hashes.
                        start_block_hash = locator.clone();
                        break;
                    }
                }

                // Collect up to GETBLOCKS_VECTORS_MAX inventory vectors for the blocks starting right
                // after the identified block on the main chain.
                let blocks = match msg.filter {
                    RequestBlockHashesFilter::ElectionOnly => blockchain
                        .get_macro_blocks(
                            &start_block_hash,
                            msg.max_blocks as u32,
                            false,
                            Direction::Forward,
                            true,
                        )
                        .unwrap(), // We made sure that start_block_hash is on our chain.
                    RequestBlockHashesFilter::All => blockchain.get_blocks(
                        &start_block_hash,
                        msg.max_blocks as u32,
                        false,
                        Direction::Forward,
                    ),
                };

                let hashes = blocks.iter().map(|block| block.hash()).collect();
                // We do not care about the result.
                let _ = peer
                    .send(&RequestResponseMessage::with_identifier(
                        BlockHashes { hashes },
                        msg.get_request_identifier(),
                    ))
                    .await;
            }
        });

        let blockchain = Arc::clone(blockchain_outer);
        let mut stream = network.receive_from_all::<RequestResponseMessage<RequestEpoch>>();
        tokio::spawn(async move {
            while let Some((msg, peer)) = stream.next().await {
                trace!(
                    "[REQUEST_EPOCH] for block {:?} received from {:?}",
                    msg.hash,
                    peer.id()
                );

                if let Some(Block::Macro(block)) = blockchain.get_block(&msg.hash, true) {
                    let epoch = policy::epoch_at(block.header.block_number);
                    let response = if let Some(transactions) =
                        blockchain.get_epoch_transactions(epoch, None)
                    {
                        Some(Epoch {
                            block,
                            transactions,
                        })
                    } else {
                        error!("Could not retrieve transactions for epoch {:?}", epoch);
                        None
                    };

                    // We need this scoping for the code to compile.
                    // Otherwise `None`, which is not Send, cannot be moved beyond the await.
                    if let Some(response) = response {
                        // We do not care about the result.
                        let _ = peer
                            .send(&RequestResponseMessage::with_identifier(
                                response,
                                msg.get_request_identifier(),
                            ))
                            .await;
                    }
                } // TODO: What to do in else case? Ignore?
            }
        });
    }
}
