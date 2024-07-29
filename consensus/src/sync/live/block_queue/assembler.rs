use std::{
    collections::HashMap,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    time::Duration,
};

use futures::{stream::BoxStream, Stream, StreamExt};
use instant::Instant;
use nimiq_block::{Block, BlockBody, MacroBlock, MicroBlock};
use nimiq_hash::{Blake2bHash, Blake2sHash, Hash};
use nimiq_network_interface::network::{CloseReason, MsgAcceptance, Network, PubsubId};
use nimiq_time::{interval, Interval};
use nimiq_utils::spawn;

use crate::{
    messages::{BlockBodyMessage, BlockBodyTopic, BlockHeaderMessage, BlockHeaderTopic},
    sync::live::block_queue::BlockSource,
};

type PubsubHeader<N> = (BlockHeaderMessage, <N as Network>::PubsubId);
type PubsubBody<N> = (BlockBodyMessage, <N as Network>::PubsubId);
type CachedBody<N> = (BlockBody, <N as Network>::PubsubId);
/// First the body root and second the header hash.
type CachedBodyKey = (Blake2sHash, Blake2bHash);

pub struct BlockAssembler<N: Network> {
    network: Arc<N>,
    header_stream: BoxStream<'static, PubsubHeader<N>>,
    body_stream: BoxStream<'static, PubsubBody<N>>,
    cached_headers: TimeLimitedCache<Blake2bHash, PubsubHeader<N>>,
    cached_bodies: TimeLimitedCache<CachedBodyKey, CachedBody<N>>,
}

impl<N: Network> BlockAssembler<N> {
    const CACHE_TTL: Duration = Duration::from_secs(10);

    pub fn new(
        network: Arc<N>,
        header_stream: BoxStream<'static, PubsubHeader<N>>,
        body_stream: BoxStream<'static, PubsubBody<N>>,
    ) -> Self {
        Self {
            network,
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

    fn reject_messages(&self, pubsub_id_header: N::PubsubId, pubsub_id_body: N::PubsubId) {
        let network = Arc::clone(&self.network);
        let peer_id_header = pubsub_id_header.propagation_source();
        let peer_id_body = pubsub_id_body.propagation_source();
        spawn(async move {
            network
                .disconnect_peer(peer_id_header, CloseReason::MaliciousPeer)
                .await;
            if peer_id_header != peer_id_body {
                network
                    .disconnect_peer(peer_id_body, CloseReason::MaliciousPeer)
                    .await;
            }
        });

        self.network
            .validate_message::<BlockHeaderTopic>(pubsub_id_header, MsgAcceptance::Reject);
        self.network
            .validate_message::<BlockBodyTopic>(pubsub_id_body, MsgAcceptance::Reject);
    }
}

impl<N: Network> Stream for BlockAssembler<N> {
    type Item = (Block, BlockSource<N>);

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        while let Poll::Ready(item) = self.header_stream.poll_next_unpin(cx) {
            let header = match item {
                Some(header) => header,
                None => return Poll::Ready(None),
            };

            let hash: Blake2bHash = header.0.hash();
            let body_key = (header.0.body_root().clone(), hash.clone());

            if let Some(body) = self.cached_bodies.remove(&body_key) {
                // Check that header and body type match.
                if header.0.ty() != body.0.ty() {
                    debug!(
                        block = %header.0,
                        peer_id_header = %header.1.propagation_source(),
                        peer_id_body = %body.1.propagation_source(),
                        "Discarding block - header and body types don't match"
                    );

                    self.reject_messages(header.1, body.1);

                    continue;
                }

                return Poll::Ready(Some((
                    Self::assemble_block(header.0, body.0),
                    BlockSource::announced(header.1, Some(body.1)),
                )));
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
                            peer_id_header = %header.1.propagation_source(),
                            peer_id_body = %body.1.propagation_source(),
                            "Discarding block - header and body types don't match"
                        );

                        self.reject_messages(header.1, body.1);

                        continue;
                    }

                    return Poll::Ready(Some((
                        Self::assemble_block(header.0, body.0.body),
                        BlockSource::announced(header.1, Some(body.1)),
                    )));
                }
            }

            let body_key = (hash, body.0.header_hash);
            let cached_body = (body.0.body, body.1);
            self.cached_bodies.insert(body_key, cached_body);
        }

        // The cache stream never returns None, so it's ok to ignore that case here.
        while let Poll::Ready(Some(evicted_entries)) = self.cached_headers.poll_next_unpin(cx) {
            for (_, header) in evicted_entries {
                trace!(header = %header.0, "Evicted header from cache");
                self.network
                    .validate_message::<BlockHeaderTopic>(header.1, MsgAcceptance::Ignore);
            }
        }
        while let Poll::Ready(Some(evicted_entries)) = self.cached_bodies.poll_next_unpin(cx) {
            let num_evicted = evicted_entries.len();
            trace!(num_evicted, "Evicted {} bodies from cache", num_evicted);

            for (_, body) in evicted_entries {
                self.network
                    .validate_message::<BlockBodyTopic>(body.1, MsgAcceptance::Ignore);
            }
        }

        Poll::Pending
    }
}

struct TimeLimitedCache<K, V> {
    map: HashMap<K, (V, Instant)>,
    ttl: Duration,
    interval: Interval,
}

impl<K: Eq + std::hash::Hash + Clone, V> TimeLimitedCache<K, V> {
    fn new(ttl: Duration) -> Self {
        Self {
            map: HashMap::default(),
            ttl,
            interval: interval(ttl),
        }
    }

    /// Returns the evicted entries.
    fn evict_expired_entries(&mut self) -> Vec<(K, V)> {
        let cutoff = Instant::now() - self.ttl;
        let keys_to_remove: Vec<K> = self
            .map
            .iter()
            .filter(|(_, v)| v.1 < cutoff)
            .map(|(k, _)| k)
            .cloned()
            .collect();

        let mut evicted_entries = Vec::with_capacity(keys_to_remove.len());
        for k in keys_to_remove {
            let v = self.map.remove(&k).unwrap();
            evicted_entries.push((k, v.0));
        }
        evicted_entries
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

impl<K: Eq + std::hash::Hash + Unpin + Clone, V: Unpin> Stream for TimeLimitedCache<K, V> {
    type Item = Vec<(K, V)>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        while self.interval.poll_next_unpin(cx).is_ready() {
            let evicted_entries = self.evict_expired_entries();
            if !evicted_entries.is_empty() {
                return Poll::Ready(Some(evicted_entries));
            }
        }
        Poll::Pending
    }
}

#[cfg(test)]
mod tests {
    use core::task::Poll;
    use std::{sync::Arc, time::Duration};

    use futures::{poll, stream::StreamExt};
    use nimiq_block::{Block, MacroBlock};
    use nimiq_network_interface::network::PubsubId;
    use nimiq_network_mock::{MockHub, MockId, MockNetwork, MockPeerId};
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

        let network = Arc::new(MockHub::new().new_network());
        let mut assembler = BlockAssembler::<MockNetwork>::new(
            network,
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
                assert_eq!(sender.peer_id(), header_sender.propagation_source());
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

        let network = Arc::new(MockHub::new().new_network());
        let mut assembler = BlockAssembler::<MockNetwork>::new(
            network,
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
                assert_eq!(sender.peer_id(), header_sender.propagation_source());
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

        assert!(matches!(
            poll!(cache.next()),
            Poll::Ready(Some(vec)) if vec == vec![(1, 1)]
        ));

        assert_eq!(cache.len(), 2);
        assert!(matches!(cache.get(&1), None));
        assert!(matches!(cache.get(&2), Some(2)));
        assert!(matches!(cache.get(&3), Some(3)));

        sleep(ttl).await;

        assert!(matches!(
            poll!(cache.next()),
            Poll::Ready(Some(vec)) if vec.len() == 2
        ));

        assert!(cache.is_empty());
    }
}
