use std::future::Future;
use std::sync::Arc;

use futures::stream::BoxStream;
use futures::StreamExt;
use parking_lot::RwLock;

use nimiq_blockchain::Blockchain;
use nimiq_network_interface::prelude::{Message, Network, Peer, ResponseMessage};

use crate::messages::handlers::Handle;
use crate::messages::{
    RequestBatchSet, RequestBlock, RequestBlockHashes, RequestHead, RequestHistoryChunk,
    RequestMissingBlocks,
};
use crate::Consensus;

impl<N: Network> Consensus<N> {
    const MAX_CONCURRENT_HANDLERS: usize = 64;

    pub(super) fn init_network_requests(network: &Arc<N>, blockchain: &Arc<RwLock<Blockchain>>) {
        let stream = network.receive_from_all::<RequestBlockHashes>();
        tokio::spawn(Self::request_handler(stream, blockchain));

        let stream = network.receive_from_all::<RequestBatchSet>();
        tokio::spawn(Self::request_handler(stream, blockchain));

        let stream = network.receive_from_all::<RequestHistoryChunk>();
        tokio::spawn(Self::request_handler(stream, blockchain));

        let stream = network.receive_from_all::<RequestBlock>();
        tokio::spawn(Self::request_handler(stream, blockchain));

        let stream = network.receive_from_all::<RequestMissingBlocks>();
        tokio::spawn(Self::request_handler(stream, blockchain));

        let stream = network.receive_from_all::<RequestHead>();
        tokio::spawn(Self::request_handler(stream, blockchain));
    }

    fn request_handler<Req: Handle<Res> + ResponseMessage, Res: Message>(
        stream: BoxStream<'static, (Req, Arc<N::PeerType>)>,
        blockchain: &Arc<RwLock<Blockchain>>,
    ) -> impl Future<Output = ()> {
        let blockchain = Arc::clone(blockchain);
        async move {
            stream
                .for_each_concurrent(Self::MAX_CONCURRENT_HANDLERS, |(msg, peer)| async {
                    let blockchain = Arc::clone(&blockchain);
                    tokio::spawn(async move {
                        trace!(
                            "[{}] {:?} {:#?}",
                            msg.get_request_identifier(),
                            peer.id(),
                            msg
                        );

                        // Try to send the response, logging to debug if it fails
                        if let Err(err) = peer.send(msg.handle(&blockchain)).await {
                            log::debug!(
                                "[{}] Failed to send {} response: {:?}",
                                msg.get_request_identifier(),
                                std::any::type_name::<Req>(),
                                err
                            );
                        };
                    })
                    .await
                    .expect("Request handler panicked")
                })
                .await
        }
    }
}
