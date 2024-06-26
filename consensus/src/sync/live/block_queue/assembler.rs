use std::{
    collections::HashMap,
    pin::Pin,
    task::{Context, Poll},
};

use futures::{stream::BoxStream, Stream, StreamExt};
use nimiq_block::{Block, BlockBody, MacroBlock, MicroBlock};
use nimiq_hash::{Blake2bHash, Blake2sHash, Hash};
use nimiq_network_interface::network::{Network, PubsubId};

use crate::messages::{BlockBodyMessage, BlockHeaderMessage};

type PubsubHeader<N> = (BlockHeaderMessage, <N as Network>::PubsubId);
type PubsubBody<N> = (BlockBodyMessage, <N as Network>::PubsubId);

pub struct BlockAssembler<N: Network> {
    header_stream: BoxStream<'static, PubsubHeader<N>>,
    body_stream: BoxStream<'static, PubsubBody<N>>,
    cached_headers: HashMap<Blake2bHash, PubsubHeader<N>>,
    cached_bodies: HashMap<Blake2sHash, HashMap<Blake2bHash, BlockBody>>,
}

impl<N: Network> BlockAssembler<N> {
    pub fn new(
        header_stream: BoxStream<'static, PubsubHeader<N>>,
        body_stream: BoxStream<'static, PubsubBody<N>>,
    ) -> Self {
        Self {
            header_stream,
            body_stream,
            cached_headers: HashMap::default(),
            cached_bodies: HashMap::default(),
        }
    }

    fn assemble_block(header: BlockHeaderMessage, body: BlockBody) -> Block {
        match header {
            BlockHeaderMessage::Macro {
                header,
                justification,
            } => Block::Macro(MacroBlock {
                header,
                body: Some(body.unwrap_macro()),
                justification: Some(justification),
            }),
            BlockHeaderMessage::Micro {
                header,
                justification,
            } => Block::Micro(MicroBlock {
                header,
                body: Some(body.unwrap_micro()),
                justification: Some(justification),
            }),
        }
    }
}

impl<N: Network> Stream for BlockAssembler<N> {
    type Item = (Block, N::PubsubId);

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        while let Poll::Ready(item) = self.header_stream.poll_next_unpin(cx) {
            let header = match item {
                Some(header) => header,
                None => return Poll::Ready(None),
            };

            let hash = header.0.hash();
            if let Some(bodies) = self.cached_bodies.get_mut(header.0.body_root()) {
                if let Some(body) = bodies.remove(&hash) {
                    if bodies.is_empty() {
                        self.cached_bodies.remove(header.0.body_root());
                    }

                    // Check that header and body type match.
                    if header.0.ty() != body.ty() {
                        debug!(
                            block = %header.0,
                            peer_id = %header.1.propagation_source(),
                            "Discarding block - header and body types don't match"
                        );
                        // TODO ban peer
                        continue;
                    }

                    return Poll::Ready(Some((Self::assemble_block(header.0, body), header.1)));
                }
            }

            self.cached_headers.insert(hash, header);
        }

        while let Poll::Ready(item) = self.body_stream.poll_next_unpin(cx) {
            let body = match item {
                Some(body) => body,
                None => return Poll::Ready(None),
            };

            let hash = body.0.body.hash();
            if let Some(header) = self.cached_headers.get(&body.0.header_hash) {
                if *header.0.body_root() == hash {
                    let header = self.cached_headers.remove(&body.0.header_hash).unwrap();

                    // Check that header and body type match.
                    if header.0.ty() != body.0.body.ty() {
                        debug!(
                            block = %header.0,
                            peer_id = %header.1.propagation_source(),
                            "Discarding block - header and body types don't match"
                        );
                        // TODO ban peer
                        continue;
                    }

                    return Poll::Ready(Some((
                        Self::assemble_block(header.0, body.0.body),
                        header.1,
                    )));
                }
            }

            self.cached_bodies
                .entry(hash)
                .or_default()
                .insert(body.0.header_hash, body.0.body);
        }

        Poll::Pending
    }
}

#[cfg(test)]
mod tests {
    use core::task::Poll;

    use futures::{poll, stream::StreamExt};
    use nimiq_block::{Block, MacroBlock};
    use nimiq_network_interface::network::PubsubId;
    use nimiq_network_mock::{MockId, MockNetwork, MockPeerId};
    use nimiq_test_log::test;
    use tokio::sync::mpsc;
    use tokio_stream::wrappers::ReceiverStream;

    use crate::{messages::BlockHeaderMessage, sync::live::block_queue::assembler::BlockAssembler};

    #[test(tokio::test)]
    async fn it_assembles_header_and_body() {
        let (header_tx, header_rx) = mpsc::channel(16);
        let (body_tx, body_rx) = mpsc::channel(16);

        let mut assembler = BlockAssembler::<MockNetwork>::new(
            ReceiverStream::new(header_rx).boxed(),
            ReceiverStream::new(body_rx).boxed(),
        );

        let block = Block::Macro(MacroBlock::non_empty_default());
        let (header, body) = BlockHeaderMessage::split_block(block.clone());

        let header_sender = MockId::new(MockPeerId::from(0));
        header_tx.try_send((header, header_sender.clone())).unwrap();

        let _ = poll!(assembler.next());
        assert_eq!(assembler.cached_headers.len(), 1);

        let body_sender = MockId::new(MockPeerId::from(1));
        body_tx.try_send((body, body_sender)).unwrap();

        match poll!(assembler.next()) {
            Poll::Ready(Some((block1, sender))) => {
                assert_eq!(block1, block);
                assert_eq!(
                    sender.propagation_source(),
                    header_sender.propagation_source()
                );
            }
            _ => panic!("Unexpected return value"),
        }

        assert!(assembler.cached_headers.is_empty());
        assert!(assembler.cached_bodies.is_empty());
    }

    #[test(tokio::test)]
    async fn it_assembles_body_and_header() {
        let (header_tx, header_rx) = mpsc::channel(16);
        let (body_tx, body_rx) = mpsc::channel(16);

        let mut assembler = BlockAssembler::<MockNetwork>::new(
            ReceiverStream::new(header_rx).boxed(),
            ReceiverStream::new(body_rx).boxed(),
        );

        let block = Block::Macro(MacroBlock::non_empty_default());
        let (header, body) = BlockHeaderMessage::split_block(block.clone());

        let body_sender = MockId::new(MockPeerId::from(1));
        body_tx.try_send((body, body_sender)).unwrap();

        let _ = poll!(assembler.next());
        assert_eq!(assembler.cached_bodies.len(), 1);
        assert_eq!(assembler.cached_bodies.values().next().unwrap().len(), 1);

        let header_sender = MockId::new(MockPeerId::from(0));
        header_tx.try_send((header, header_sender.clone())).unwrap();

        match poll!(assembler.next()) {
            Poll::Ready(Some((block1, sender))) => {
                assert_eq!(block1, block);
                assert_eq!(
                    sender.propagation_source(),
                    header_sender.propagation_source()
                );
            }
            _ => panic!("Unexpected return value"),
        }

        assert!(assembler.cached_headers.is_empty());
        assert!(assembler.cached_bodies.is_empty());
    }
}
