use std::time::Duration;

use libp2p::gossipsub::TopicHash;
use prometheus_client::{
    encoding::EncodeLabelSet,
    metrics::{counter::Counter, family::Family, histogram::Histogram},
    registry::Registry,
};

pub struct NetworkMetrics {
    gossipsub_messages_received: Family<TopicLabels, Counter>,
    gossipsub_messages_published: Family<TopicLabels, Counter>,
    response_times: Histogram,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
struct TopicLabels {
    topic: String,
}

impl Default for NetworkMetrics {
    fn default() -> Self {
        NetworkMetrics {
            gossipsub_messages_received: Default::default(),
            gossipsub_messages_published: Default::default(),
            // Up to the request timeout (10 s, see `REQUEST_TIMEOUT`), so slow responses are
            // not all lumped into the `+Inf` bucket.
            response_times: Histogram::new([0.01, 0.02, 0.05, 0.1, 0.2, 0.5, 1.0, 2.0, 5.0, 10.0]),
        }
    }
}

/// Returns the topic without its subtopic. Subtopics are named `<topic>_<subtopic>` (see
/// `Network::subscribe_subtopic`), e.g. one per address for address subscriptions, which would
/// make an unbounded label. Gossipsub only delivers messages of subscribed topics, so the base
/// topic is always one of our own topic names and the label stays bounded.
fn base_topic(topic: &str) -> &str {
    topic
        .split_once('_')
        .map_or(topic, |(base, _subtopic)| base)
}

impl NetworkMetrics {
    pub fn register(&self, registry: &mut Registry) {
        registry.register(
            "gossipsub_messages_received",
            "Number of received gossipsub messages, before validation",
            self.gossipsub_messages_received.clone(),
        );

        registry.register(
            "gossipsub_messages_published",
            "Number of published gossipsub messages",
            self.gossipsub_messages_published.clone(),
        );

        registry.register(
            "request_durations",
            "Time between requests and responses",
            self.response_times.clone(),
        );
    }

    pub(crate) fn note_received_pubsub_message(&self, topic: &TopicHash) {
        self.gossipsub_messages_received
            .get_or_create(&TopicLabels {
                topic: base_topic(topic.as_str()).to_owned(),
            })
            .inc();
    }

    pub(crate) fn note_published_pubsub_message(&self, topic_str: &str) {
        self.gossipsub_messages_published
            .get_or_create(&TopicLabels {
                topic: String::from(topic_str),
            })
            .inc();
    }

    pub(crate) fn note_response_time(&self, duration: Duration) {
        self.response_times.observe(duration.as_secs_f64());
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use libp2p::gossipsub::TopicHash;
    use prometheus_client::{encoding::text::encode, registry::Registry};

    use super::NetworkMetrics;

    #[test]
    fn labels_received_messages_by_base_topic() {
        let metrics = NetworkMetrics::default();
        let mut registry = Registry::default();
        metrics.register(&mut registry);

        metrics.note_received_pubsub_message(&TopicHash::from_raw("block-header"));
        metrics.note_received_pubsub_message(&TopicHash::from_raw(
            "address-subscription_12D3KooWExamplePeer",
        ));
        metrics.note_received_pubsub_message(&TopicHash::from_raw(
            "address-subscription_12D3KooWOtherPeer",
        ));

        let mut encoded = String::new();
        encode(&mut encoded, &registry).unwrap();
        assert!(encoded.contains(r#"gossipsub_messages_received_total{topic="block-header"} 1"#));
        assert!(encoded
            .contains(r#"gossipsub_messages_received_total{topic="address-subscription"} 2"#));
        assert!(!encoded.contains("12D3KooW"));
        assert!(!encoded.contains(r#"topic="unknown""#));
    }

    #[test]
    fn buckets_response_times_up_to_the_request_timeout() {
        let metrics = NetworkMetrics::default();
        let mut registry = Registry::default();
        metrics.register(&mut registry);

        metrics.note_response_time(Duration::from_secs(7));

        let mut encoded = String::new();
        encode(&mut encoded, &registry).unwrap();
        assert!(encoded.contains(r#"request_durations_bucket{le="5.0"} 0"#));
        assert!(encoded.contains(r#"request_durations_bucket{le="10.0"} 1"#));
    }
}
