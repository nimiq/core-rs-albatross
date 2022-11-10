use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use futures::future::BoxFuture;
use futures::{FutureExt, Stream, StreamExt};

use nimiq_block::MacroBlock;
use nimiq_network_interface::network::Network;
use nimiq_network_interface::request::RequestError;

use crate::types::*;
use futures::stream::FuturesUnordered;

/// This component handles the requests to a given set of peers.
///
/// This component has:
/// - Network
/// - The list of futures of replies from peers
///
/// Polling this gives the next zkp response we received from our peers.
pub struct ZKPRequests<TNetwork: Network + 'static> {
    network: Arc<TNetwork>,
    zkp_request_results: FuturesUnordered<
        BoxFuture<
            'static,
            (
                TNetwork::PeerId,
                bool,
                Result<(Option<ZKProof>, Option<MacroBlock>), RequestError>,
            ),
        >,
    >,
}

impl<TNetwork: Network + 'static> ZKPRequests<TNetwork> {
    pub fn new(network: Arc<TNetwork>) -> Self {
        ZKPRequests {
            network,
            zkp_request_results: FuturesUnordered::new(),
        }
    }

    /// The request zkps is finished once responses were received.
    pub fn is_finished(&self) -> bool {
        self.zkp_request_results.is_empty()
    }

    /// Created the futures to requests zkps to all specified peers.
    pub fn request_zkps(
        &mut self,
        peers: Vec<TNetwork::PeerId>,
        block_number: u32,
        request_election_block: bool,
    ) {
        for peer_id in peers {
            let network = Arc::clone(&self.network);
            self.zkp_request_results.push(
                async move {
                    (
                        peer_id,
                        request_election_block,
                        network
                            .request::<RequestZKP>(
                                RequestZKP {
                                    block_number,
                                    request_election_block,
                                },
                                peer_id,
                            )
                            .await,
                    )
                }
                .boxed(),
            );
        }
    }
}

impl<TNetwork: Network + 'static> Stream for ZKPRequests<TNetwork> {
    type Item = (TNetwork::PeerId, ZKProof, Option<MacroBlock>);

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        // We poll the zkp requests and return the proof.
        while let Poll::Ready(result) = self.zkp_request_results.poll_next_unpin(cx) {
            match result {
                Some((peer_id, request_election_block, result)) => match result {
                    Ok((Some(proof), mut election_block)) => {
                        // Check that the response is in-line with whether we asked for the election block or not.
                        if request_election_block {
                            if election_block.is_none() {
                                continue;
                            }
                        } else {
                            election_block = None;
                        }
                        return Poll::Ready(Some((peer_id, proof, election_block)));
                    }
                    Ok((None, _)) => {
                        // This happens when the peer does not have a more recent proof than us.
                    }
                    Err(_) => {
                        log::trace!("Failed zkp request");
                    }
                },
                None => return Poll::Ready(None),
            }
        }
        Poll::Pending
    }
}
