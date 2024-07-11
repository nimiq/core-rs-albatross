use std::{
    pin::Pin,
    sync::Arc,
    task::{Context, Poll, Waker},
};

use futures::{future::BoxFuture, stream::FuturesUnordered, FutureExt, Stream, StreamExt};
use nimiq_block::MacroBlock;
use nimiq_network_interface::{network::Network, request::RequestError};
use nimiq_utils::WakerExt as _;
use tokio::sync::oneshot::{channel, Receiver, Sender};

use crate::types::*;

pub struct ZKPRequestsItem<N: Network> {
    pub peer_id: N::PeerId,
    pub proof: ZKProof,
    pub election_block: Option<MacroBlock>,
    pub response_channel: Option<Sender<Result<ZKPRequestEvent, Error>>>,
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
    pub(crate) waker: Option<Waker>,
    network: Arc<N>,
    zkp_request_results: FuturesUnordered<
        BoxFuture<
            'static,
            (
                N::PeerId,
                bool,
                Option<Sender<Result<ZKPRequestEvent, Error>>>,
                Result<RequestZKPResponse, RequestError>,
            ),
        >,
    >,
}

impl<N: Network + 'static> ZKPRequests<N> {
    pub fn new(network: Arc<N>) -> Self {
        ZKPRequests {
            waker: None,
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
    ) -> Receiver<Result<ZKPRequestEvent, Error>> {
        let (tx, rx) = channel();
        self.push_request(peer_id, block_number, request_election_block, Some(tx));

        rx
    }

    fn push_request(
        &mut self,
        peer_id: N::PeerId,
        block_number: u32,
        request_election_block: bool,
        response_channel: Option<Sender<Result<ZKPRequestEvent, Error>>>,
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
        // Pushing to the futures unordered above does not wake the task that polls zkp_requests_results
        // So we need to wake the task manually
        self.waker.wake();
    }
}

impl<N: Network + 'static> Stream for ZKPRequests<N> {
    type Item = ZKPRequestsItem<N>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.waker.store_waker(cx);

        // We poll the zkp requests and return the proof.
        while let Poll::Ready(result) = self.zkp_request_results.poll_next_unpin(cx) {
            match result {
                Some((peer_id, request_election_block, response_channel, result)) => match result {
                    Ok(RequestZKPResponse::Proof(proof, mut election_block)) => {
                        // Check that the response is in-line with whether we asked for the election block or not.
                        if request_election_block {
                            if election_block.is_none() {
                                if let Some(tx) = response_channel {
                                    let _ = tx.send(Err(Error::InvalidBlock));
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
                    Ok(RequestZKPResponse::Outdated(block_height)) => {
                        // This happens when the peer does not have a more recent proof than us.
                        if let Some(tx) = response_channel {
                            let _ = tx.send(Ok(ZKPRequestEvent::OutdatedProof { block_height }));
                        }
                    }
                    Err(e) => {
                        log::trace!("Failed zkp request");
                        if let Some(tx) = response_channel {
                            let _ = tx.send(Err(Error::Request(e)));
                        }
                    }
                },
                None => return Poll::Ready(None),
            }
        }
        Poll::Pending
    }
}
