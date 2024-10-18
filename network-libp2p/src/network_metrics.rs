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
            response_times: Histogram::new([0.01, 0.02, 0.05, 0.1, 0.2, 0.5, 1.0, 2.0].into_iter()),
        }
    }
}

impl NetworkMetrics {
    pub fn register(&self, registry: &mut Registry) {
        registry.register(
            "gossipsub_messages_received",
            "Number of received gossipsub messages",
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
        if [
            "address-subscription",
            "block-body",
            "block-header",
            "control-transaction",
            "regular-transaction",
            "tendermint-proposal",
            "zk-proof",
        ]
        .contains(&&*topic.to_string())
        {
            self.gossipsub_messages_received
                .get_or_create(&TopicLabels {
                    topic: topic.to_string(),
                })
                .inc();
        } else {
            self.gossipsub_messages_received
                .get_or_create(&TopicLabels {
                    topic: "unknown".into(),
                })
                .inc();
        }
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
