use std::{
    collections::HashMap,
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};

use futures::{stream::BoxStream, Stream, StreamExt};
use instant::Instant;
use nimiq_block::{Block, BlockBody, MacroBlock, MicroBlock};
use nimiq_hash::{Blake2bHash, Blake2sHash, Hash};
use nimiq_network_interface::network::{Network, PubsubId};
use nimiq_time::{interval, Interval};

use crate::messages::{BlockBodyMessage, BlockHeaderMessage};

type PubsubHeader<N> = (BlockHeaderMessage, <N as Network>::PubsubId);
type PubsubBody<N> = (BlockBodyMessage, <N as Network>::PubsubId);
type BodyKey = (Blake2sHash, Blake2bHash);

pub struct BlockAssembler<N: Network> {
    header_stream: BoxStream<'static, PubsubHeader<N>>,
    body_stream: BoxStream<'static, PubsubBody<N>>,
    cached_headers: TimeLimitedCache<Blake2bHash, PubsubHeader<N>>,
    cached_bodies: TimeLimitedCache<BodyKey, BlockBody>,
}

impl<N: Network> BlockAssembler<N> {
    const CACHE_TTL: Duration = Duration::from_secs(5);

    pub fn new(
        header_stream: BoxStream<'static, PubsubHeader<N>>,
        body_stream: BoxStream<'static, PubsubBody<N>>,
    ) -> Self {
        Self {
            header_stream,
            body_stream,
            cached_headers: TimeLimitedCache::new(Self::CACHE_TTL),
            cached_bodies: TimeLimitedCache::new(Self::CACHE_TTL),
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
        // The cache stream never returns None, so it's ok to ignore that case here.
        while let Poll::Ready(Some(num_evicted)) = self.cached_headers.poll_next_unpin(cx) {
            debug!(num_evicted, "Evicted {} headers from cache", num_evicted);
        }
        while let Poll::Ready(Some(num_evicted)) = self.cached_bodies.poll_next_unpin(cx) {
            debug!(num_evicted, "Evicted {} bodies from cache", num_evicted);
        }

        while let Poll::Ready(item) = self.header_stream.poll_next_unpin(cx) {
            let header = match item {
                Some(header) => header,
                None => return Poll::Ready(None),
            };

            let hash: Blake2bHash = header.0.hash();
            let body_key = (header.0.body_root().clone(), hash.clone());

            if let Some(body) = self.cached_bodies.remove(&body_key) {
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

            let body_key = (hash, body.0.header_hash);
            self.cached_bodies.insert(body_key, body.0.body);
        }

        Poll::Pending
    }
}

struct TimeLimitedCache<K, V> {
    map: HashMap<K, (V, Instant)>,
    ttl: Duration,
    interval: Interval,
}

impl<K: Eq + std::hash::Hash, V> TimeLimitedCache<K, V> {
    fn new(ttl: Duration) -> Self {
        Self {
            map: HashMap::default(),
            ttl,
            interval: interval(ttl),
        }
    }

    /// Returns the number of items that were evicted.
    fn evict_expired_entries(&mut self) -> usize {
        let initial_len = self.map.len();

        let cutoff = Instant::now() - self.ttl;
        self.map
            .retain(|_, (_, insertion_time)| *insertion_time >= cutoff);

        initial_len - self.map.len()
    }

    fn get(&self, k: &K) -> Option<&V> {
        self.map.get(k).map(|v| &v.0)
    }

    fn insert(&mut self, k: K, v: V) -> Option<V> {
        self.map.insert(k, (v, Instant::now())).map(|v| v.0)
    }

    fn remove(&mut self, k: &K) -> Option<V> {
        self.map.remove(k).map(|v| v.0)
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.map.len()
    }

    #[cfg(test)]
    fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

impl<K: Eq + std::hash::Hash + Unpin, V: Unpin> Stream for TimeLimitedCache<K, V> {
    type Item = usize;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        while self.interval.poll_next_unpin(cx).is_ready() {
            let num_evicted = self.evict_expired_entries();
            if num_evicted > 0 {
                return Poll::Ready(Some(num_evicted));
            }
        }
        Poll::Pending
    }
}

#[cfg(test)]
mod tests {
    use core::task::Poll;
    use std::time::Duration;

    use futures::{poll, stream::StreamExt};
    use nimiq_block::{Block, MacroBlock};
    use nimiq_network_interface::network::PubsubId;
    use nimiq_network_mock::{MockId, MockNetwork, MockPeerId};
    use nimiq_test_log::test;
    use nimiq_time::sleep;
    use tokio::sync::mpsc;
    use tokio_stream::wrappers::ReceiverStream;

    use crate::{
        messages::BlockHeaderMessage,
        sync::live::block_queue::assembler::{BlockAssembler, TimeLimitedCache},
    };

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

    #[test(tokio::test)]
    async fn it_evicts_expired_items() {
        let ttl = Duration::from_secs(1);
        let mut cache = TimeLimitedCache::new(ttl);

        cache.insert(1, 1);

        sleep(ttl / 2).await;

        assert!(matches!(poll!(cache.next()), Poll::Pending));

        assert_eq!(cache.len(), 1);
        assert!(matches!(cache.get(&1), Some(1)));

        cache.insert(2, 2);
        cache.insert(3, 3);

        sleep(3 * ttl / 4).await;

        assert!(matches!(poll!(cache.next()), Poll::Ready(Some(1))));

        assert_eq!(cache.len(), 2);
        assert!(matches!(cache.get(&1), None));
        assert!(matches!(cache.get(&2), Some(2)));
        assert!(matches!(cache.get(&3), Some(3)));

        sleep(ttl).await;

        assert!(matches!(poll!(cache.next()), Poll::Ready(Some(2))));

        assert!(cache.is_empty());
    }
}
