use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use futures::future::BoxFuture;
use futures::{FutureExt, Stream, StreamExt};

use nimiq_network_interface::request::RequestError;
use tokio::sync::oneshot::{channel, Receiver, Sender};

use nimiq_block::MacroBlock;
use nimiq_network_interface::network::Network;

use crate::types::*;
use futures::stream::FuturesUnordered;

pub struct ZKPRequestsItem<N: Network> {
    pub peer_id: N::PeerId,
    pub proof: ZKProof,
    pub election_block: Option<MacroBlock>,
    pub response_channel: Option<Sender<Result<ZKPRequestEvent<N>, Error>>>,
}

/// This component handles the requests to a given set of peers.
///
/// This component has:
/// - Network
/// - The list of futures of replies from peers
///
/// Polling this gives the next zkp response we received from our peers.
///
/// We offer the option to receive back the verification result via a one-shot channel.
pub struct ZKPRequests<N: Network + 'static> {
    network: Arc<N>,
    zkp_request_results: FuturesUnordered<
        BoxFuture<
            'static,
            (
                N::PeerId,
                bool,
                Option<Sender<Result<ZKPRequestEvent<N>, Error>>>,
                Result<(Option<ZKProof>, Option<MacroBlock>), RequestError>,
            ),
        >,
    >,
}

impl<N: Network + 'static> ZKPRequests<N> {
    pub fn new(network: Arc<N>) -> Self {
        ZKPRequests {
            network,
            zkp_request_results: FuturesUnordered::new(),
        }
    }

    /// The request zkps is finished once responses were received.
    pub fn is_finished(&self) -> bool {
        self.zkp_request_results.is_empty()
    }

    /// Creates the futures to request zkps from all specified peers.
    pub fn request_zkps(
        &mut self,
        peers: Vec<N::PeerId>,
        block_number: u32,
        request_election_block: bool,
    ) {
        for peer_id in peers {
            self.push_request(peer_id, block_number, request_election_block, None);
        }
    }

    /// Requests a ZKP from a single peer and return future with verification result.
    pub fn request_zkp(
        &mut self,
        peer_id: N::PeerId,
        block_number: u32,
        request_election_block: bool,
    ) -> Receiver<Result<ZKPRequestEvent<N>, Error>> {
        let (tx, rx) = channel();
        self.push_request(peer_id, block_number, request_election_block, Some(tx));

        rx
    }

    fn push_request(
        &mut self,
        peer_id: N::PeerId,
        block_number: u32,
        request_election_block: bool,
        response_channel: Option<Sender<Result<ZKPRequestEvent<N>, Error>>>,
    ) {
        let network = Arc::clone(&self.network);
        self.zkp_request_results.push(
            async move {
                (
                    peer_id,
                    request_election_block,
                    response_channel,
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

impl<N: Network + 'static> Stream for ZKPRequests<N> {
    type Item = ZKPRequestsItem<N>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        // We poll the zkp requests and return the proof.
        while let Poll::Ready(result) = self.zkp_request_results.poll_next_unpin(cx) {
            match result {
                Some((peer_id, request_election_block, response_channel, result)) => match result {
                    Ok((Some(proof), mut election_block)) => {
                        // Check that the response is in-line with whether we asked for the election block or not.
                        if request_election_block {
                            if election_block.is_none() {
                                if let Some(rx) = response_channel {
                                    let _ = rx.send(Err(Error::InvalidBlock));
                                }
                                continue;
                            }
                        } else {
                            election_block = None;
                        }
                        return Poll::Ready(Some(ZKPRequestsItem {
                            peer_id,
                            proof,
                            election_block,
                            response_channel,
                        }));
                    }
                    Ok((None, _)) => {
                        // This happens when the peer does not have a more recent proof than us.
                        if let Some(rx) = response_channel {
                            let _ = rx.send(Ok(ZKPRequestEvent::OutdatedProof));
                        }
                    }
                    Err(e) => {
                        log::trace!("Failed zkp request");
                        if let Some(rx) = response_channel {
                            let _ = rx.send(Err(Error::Request(e)));
                        }
                    }
                },
                None => return Poll::Ready(None),
            }
        }
        Poll::Pending
    }
}
