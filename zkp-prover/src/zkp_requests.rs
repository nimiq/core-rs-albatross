use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use futures::future::BoxFuture;
use futures::{Future, FutureExt, StreamExt};
use nimiq_network_interface::network::Network;
use nimiq_network_interface::request::RequestError;

use crate::types::*;
use futures::stream::FuturesUnordered;

pub struct ZKPRequests<TNetwork: Network + 'static> {
    peers: Vec<TNetwork::PeerId>,
    zkp_request_results:
        FuturesUnordered<BoxFuture<'static, (usize, Result<Option<ZKProof>, RequestError>)>>,
}

impl<TNetwork: Network + 'static> ZKPRequests<TNetwork> {
    pub fn new(network: Arc<TNetwork>, block_number: u32) -> Self {
        let peers = network.get_peers();
        let zkp_request_resuls = peers
            .iter()
            .enumerate()
            .map(|(i, peer_id)| {
                let peer_id = *peer_id;
                let network = Arc::clone(&network);
                async move { (i, Self::request_zkp(network, peer_id, block_number).await) }.boxed()
            })
            .collect();

        ZKPRequests {
            peers,
            zkp_request_results: zkp_request_resuls,
        }
    }

    pub fn is_finished(&self) -> bool {
        self.zkp_request_results.is_empty()
    }

    async fn request_zkp(
        network: Arc<TNetwork>,
        peer_id: TNetwork::PeerId,
        block_number: u32,
    ) -> Result<Option<ZKProof>, RequestError> {
        network
            .request::<RequestZKP>(RequestZKP { block_number }, peer_id)
            .await
    }
}

impl<TNetwork: Network + 'static> Future for ZKPRequests<TNetwork> {
    type Output = (TNetwork::PeerId, ZKProof);

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // We poll the zkp requests.
        while let Poll::Ready(Some((i, result))) = self.zkp_request_results.poll_next_unpin(cx) {
            // If we got a result, check it and try to push it in to the zkp state.
            match result {
                Ok(Some(proof)) => {
                    let peer_id = self.peers[i];

                    return Poll::Ready((peer_id, proof));
                }
                Ok(None) => {}
                Err(_) => {
                    log::trace!("Failed zkp request");
                }
            }
        }
        Poll::Pending
    }
}
