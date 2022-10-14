use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use futures::future::BoxFuture;
use futures::{FutureExt, Stream, StreamExt};
use nimiq_network_interface::network::Network;
use nimiq_network_interface::request::RequestError;

use crate::types::*;
use futures::stream::FuturesUnordered;

pub struct ZKPRequests<TNetwork: Network + 'static> {
    network: Arc<TNetwork>,
    zkp_request_results: FuturesUnordered<
        BoxFuture<'static, (TNetwork::PeerId, Result<Option<ZKProof>, RequestError>)>,
    >,
}

impl<TNetwork: Network + 'static> ZKPRequests<TNetwork> {
    pub fn new(network: Arc<TNetwork>) -> Self {
        ZKPRequests {
            network,
            zkp_request_results: FuturesUnordered::new(),
        }
    }

    pub fn is_finished(&self) -> bool {
        self.zkp_request_results.is_empty()
    }

    pub fn request_zkps(&mut self, peers: Vec<TNetwork::PeerId>, block_number: u32) {
        for peer_id in peers {
            let network = Arc::clone(&self.network);
            self.zkp_request_results.push(
                async move {
                    (
                        peer_id,
                        network
                            .request::<RequestZKP>(RequestZKP { block_number }, peer_id)
                            .await,
                    )
                }
                .boxed(),
            );
        }
    }
}

impl<TNetwork: Network + 'static> Stream for ZKPRequests<TNetwork> {
    type Item = (TNetwork::PeerId, ZKProof);

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        // We poll the zkp requests.
        while let Poll::Ready(result) = self.zkp_request_results.poll_next_unpin(cx) {
            match result {
                Some((peer_id, result)) => match result {
                    Ok(Some(proof)) => {
                        return Poll::Ready(Some((peer_id, proof)));
                    }
                    Ok(None) => {}
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
