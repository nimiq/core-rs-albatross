use std::{
    collections::HashMap,
    fmt,
    sync::{atomic::Ordering, Arc},
};

use metrics::{
    atomics::AtomicU64, Counter, CounterFn, Gauge, GaugeFn, Histogram, HistogramFn, Key, KeyName,
    Label, Metadata, Recorder, SharedString, Unit,
};
use parking_lot::RwLock;
use prometheus_client::{
    collector::Collector,
    encoding::{DescriptorEncoder, EncodeMetric, MetricEncoder},
    metrics::MetricType,
    registry::Unit as PrometheusUnit,
};

const HIST_BUCKETS: [f64; 11] = [
    0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0,
];

#[derive(Debug, Default)]
struct Descriptor {
    help: String,
    unit: Option<PrometheusUnit>,
}

impl Descriptor {
    fn new(help: String, unit: Option<Unit>) -> Self {
        Self {
            help,
            unit: unit.map(convert_unit_to_prometheus),
        }
    }
}

#[derive(Debug)]
enum Metric {
    Counter(Arc<MetricsCounter>),
    Gauge(Arc<MetricsGauge>),
    Histogram(Arc<MetricsHistogram>),
}

impl EncodeMetric for Metric {
    fn encode(&self, encoder: MetricEncoder) -> Result<(), fmt::Error> {
        match self {
            Metric::Counter(counter) => counter.inner.encode(encoder),
            Metric::Gauge(gauge) => gauge.inner.encode(encoder),
            Metric::Histogram(hist) => hist.inner.encode(encoder),
        }
    }

    fn metric_type(&self) -> MetricType {
        match self {
            Metric::Counter(_) => MetricType::Counter,
            Metric::Gauge(_) => MetricType::Gauge,
            Metric::Histogram(_) => MetricType::Histogram,
        }
    }
}

/// This module provides compatibility with the `metrics` crate.
/// It registers as a `Collector` with the `prometheus_client` crate
/// and implements the `Recorder` trait of the `metrics` crate.
#[derive(Debug, Default)]
pub struct MetricsCollector {
    metrics: Arc<RwLock<HashMap<KeyName, Vec<(Vec<(String, String)>, Metric)>>>>,
    descriptors: Arc<RwLock<HashMap<KeyName, Descriptor>>>,
}

impl Clone for MetricsCollector {
    fn clone(&self) -> Self {
        Self {
            metrics: Arc::clone(&self.metrics),
            descriptors: Arc::clone(&self.descriptors),
        }
    }
}

impl Collector for MetricsCollector {
    fn encode(&self, mut encoder: DescriptorEncoder) -> Result<(), fmt::Error> {
        for (key_name, metrics) in self.metrics.read().iter() {
            // Find descriptor for the metric.
            let descriptors = self.descriptors.read();
            let (help, unit) = descriptors
                .get(key_name)
                .map(|d| (d.help.as_str(), d.unit.as_ref()))
                .unwrap_or_else(|| ("", None));

            // Gather statistics about the metrics.
            if metrics.is_empty() {
                continue;
            }
            let metric_type = metrics[0].1.metric_type();
            // If there is more than one entry, this is always true.
            let has_labels = !metrics[0].0.is_empty();

            // Encode descriptor and metric.
            let mut descriptor_encoder =
                encoder.encode_descriptor(key_name.as_str(), help, unit, metric_type)?;

            // Encode metrics for this key.
            // If labels are present, encode the metric as a family.
            if has_labels {
                for (labels, metric) in metrics {
                    let metric_encoder = descriptor_encoder.encode_family(labels)?;
                    metric.encode(metric_encoder)?;
                }
            } else {
                let metric = &metrics[0].1;
                metric.encode(descriptor_encoder)?;
            }
        }
        Ok(())
    }
}

impl MetricsCollector {
    fn register(&self, key: &Key, metric: Metric) {
        let (key_name, labels) = key.clone().into_parts();
        let labels = convert_labels_to_prometheus(labels);

        let mut metrics = self.metrics.write();
        let entry = metrics.entry(key_name).or_default();

        // Make sure that all metrics for a key have the same type
        // and that labels are set on duplicate entries..
        assert!(
            entry.is_empty()
                || (entry[0].1.metric_type().as_str() == metric.metric_type().as_str()
                    && !entry[0].0.is_empty()
                    && !labels.is_empty()),
            "Registering a metric with a different type or missing labels: `{:?}`",
            key
        );
        entry.push((labels, metric));
    }

    fn describe(&self, key: &KeyName, unit: Option<Unit>, description: SharedString) {
        assert!(
            self.descriptors
                .write()
                .insert(key.clone(), Descriptor::new(description.into_owned(), unit))
                .is_none(),
            "Registering a duplicate metric descriptor: `{:?}`",
            key
        );
    }
}

impl Recorder for MetricsCollector {
    fn describe_counter(&self, key: KeyName, unit: Option<Unit>, description: SharedString) {
        self.describe(&key, unit, description)
    }

    fn describe_gauge(&self, key: KeyName, unit: Option<Unit>, description: SharedString) {
        self.describe(&key, unit, description)
    }

    fn describe_histogram(&self, key: KeyName, unit: Option<Unit>, description: SharedString) {
        self.describe(&key, unit, description)
    }

    fn register_counter(&self, key: &Key, _metadata: &Metadata<'_>) -> Counter {
        let counter = Arc::new(MetricsCounter::default());
        self.register(key, Metric::Counter(Arc::clone(&counter)));
        Counter::from_arc(counter)
    }

    fn register_gauge(&self, key: &Key, _metadata: &Metadata<'_>) -> Gauge {
        let gauge = Arc::new(MetricsGauge::default());
        self.register(key, Metric::Gauge(Arc::clone(&gauge)));
        Gauge::from_arc(gauge)
    }

    fn register_histogram(&self, key: &Key, _metadata: &Metadata<'_>) -> Histogram {
        let hist = Arc::new(MetricsHistogram::default());
        self.register(key, Metric::Histogram(Arc::clone(&hist)));
        Histogram::from_arc(hist)
    }
}

fn convert_labels_to_prometheus(labels: Vec<Label>) -> Vec<(String, String)> {
    labels
        .into_iter()
        .map(Label::into_parts)
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect()
}

fn convert_unit_to_prometheus(unit: Unit) -> PrometheusUnit {
    match unit {
        Unit::Count => PrometheusUnit::Other("count".to_string()),
        Unit::Percent => PrometheusUnit::Other("percent".to_string()),
        Unit::Seconds => PrometheusUnit::Other("seconds".to_string()),
        Unit::Milliseconds => PrometheusUnit::Other("milliseconds".to_string()),
        Unit::Microseconds => PrometheusUnit::Other("microseconds".to_string()),
        Unit::Nanoseconds => PrometheusUnit::Other("nanoseconds".to_string()),
        Unit::Tebibytes => PrometheusUnit::Other("tebibytes".to_string()),
        Unit::Gigibytes => PrometheusUnit::Other("gibibytes".to_string()),
        Unit::Mebibytes => PrometheusUnit::Other("mebibytes".to_string()),
        Unit::Kibibytes => PrometheusUnit::Other("kibibytes".to_string()),
        Unit::Bytes => PrometheusUnit::Bytes,
        Unit::TerabitsPerSecond => PrometheusUnit::Other("terabits per second".to_string()),
        Unit::GigabitsPerSecond => PrometheusUnit::Other("gigabits per second".to_string()),
        Unit::MegabitsPerSecond => PrometheusUnit::Other("megabits per second".to_string()),
        Unit::KilobitsPerSecond => PrometheusUnit::Other("kilobits per second".to_string()),
        Unit::BitsPerSecond => PrometheusUnit::Other("bits per second".to_string()),
        Unit::CountPerSecond => PrometheusUnit::Other("count per second".to_string()),
    }
}

#[derive(Debug, Default)]
struct MetricsCounter {
    inner: prometheus_client::metrics::counter::Counter,
}

impl CounterFn for MetricsCounter {
    fn increment(&self, value: u64) {
        self.inner.inc_by(value);
    }

    fn absolute(&self, value: u64) {
        self.inner.inner().fetch_max(value, Ordering::Relaxed);
    }
}

#[derive(Debug, Default)]
struct MetricsGauge {
    inner: prometheus_client::metrics::gauge::Gauge<f64, AtomicU64>,
}

impl GaugeFn for MetricsGauge {
    fn increment(&self, value: f64) {
        self.inner.inc_by(value);
    }

    fn decrement(&self, value: f64) {
        self.inner.dec_by(value);
    }

    fn set(&self, value: f64) {
        self.inner.set(value);
    }
}

#[derive(Debug)]
struct MetricsHistogram {
    inner: prometheus_client::metrics::histogram::Histogram,
}

impl Default for MetricsHistogram {
    fn default() -> Self {
        // We currently hardcode the buckets.
        Self {
            inner: prometheus_client::metrics::histogram::Histogram::new(HIST_BUCKETS.into_iter()),
        }
    }
}

impl HistogramFn for MetricsHistogram {
    fn record(&self, value: f64) {
        self.inner.observe(value);
    }
}
