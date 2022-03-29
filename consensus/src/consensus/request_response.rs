use std::future::Future;
use std::sync::Arc;

use futures::stream::BoxStream;
use futures::StreamExt;
use parking_lot::RwLock;

use nimiq_blockchain::Blockchain;
use nimiq_network_interface::prelude::{Message, Network};

use crate::messages::handlers::Handle;
use crate::messages::{
    RequestBatchSet, RequestBlock, RequestHead, RequestHistoryChunk, RequestMacroChain,
    RequestMissingBlocks,
};
use crate::Consensus;

impl<N: Network> Consensus<N> {
    const MAX_CONCURRENT_HANDLERS: usize = 64;

    pub(super) fn init_network_request_receivers(
        network: &Arc<N>,
        blockchain: &Arc<RwLock<Blockchain>>,
    ) {
        let stream = network.receive_requests::<RequestMacroChain>();
        tokio::spawn(Self::request_handler(network, stream, blockchain));

        let stream = network.receive_requests::<RequestBatchSet>();
        tokio::spawn(Self::request_handler(network, stream, blockchain));

        let stream = network.receive_requests::<RequestHistoryChunk>();
        tokio::spawn(Self::request_handler(network, stream, blockchain));

        let stream = network.receive_requests::<RequestBlock>();
        tokio::spawn(Self::request_handler(network, stream, blockchain));

        let stream = network.receive_requests::<RequestMissingBlocks>();
        tokio::spawn(Self::request_handler(network, stream, blockchain));

        let stream = network.receive_requests::<RequestHead>();
        tokio::spawn(Self::request_handler(network, stream, blockchain));
    }

    pub(crate) fn request_handler<Req: Handle<Res> + Message, Res: Message>(
        network: &Arc<N>,
        stream: BoxStream<'static, (Req, N::RequestId, N::PeerId)>,
        blockchain: &Arc<RwLock<Blockchain>>,
    ) -> impl Future<Output = ()> {
        let blockchain = Arc::clone(blockchain);
        let network = Arc::clone(network);
        async move {
            stream
                .for_each_concurrent(
                    Self::MAX_CONCURRENT_HANDLERS,
                    |(msg, request_id, peer_id)| {
                        let request_id = request_id;
                        let network = Arc::clone(&network);
                        let blockchain = Arc::clone(&blockchain);
                        async move {
                            let blockchain = Arc::clone(&blockchain);
                            let network = Arc::clone(&network);
                            let request_id = request_id;
                            tokio::spawn(async move {
                                trace!("[{:?}] {:?} {:#?}", request_id, peer_id, msg);

                                // Try to send the response, logging to debug if it fails
                                if let Err(err) =
                                    network.respond(request_id, msg.handle(&blockchain)).await
                                {
                                    log::debug!(
                                        "[{:?}] Failed to send {} response: {:?}",
                                        request_id,
                                        std::any::type_name::<Req>(),
                                        err
                                    );
                                };
                            })
                            .await
                            .expect("Request handler panicked")
                        }
                    },
                )
                .await
        }
    }
}
