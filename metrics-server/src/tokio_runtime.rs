use std::{collections::HashMap, sync::Arc, time::Duration};

use parking_lot::RwLock;
use prometheus_client::{metrics::gauge::Gauge, registry::Registry};
use tokio_metrics::{RuntimeMetrics, RuntimeMonitor};

const TOKIO_METRICS_FREQ_SECS: u64 = 1;

static TOKIO_RT_METRICS_NAME: &[&str] = &[
    "worker_count",
    "total_park_count",
    "max_park_count",
    "min_park_count",
    "total_noop_count",
    "max_noop_count",
    "min_noop_count",
    "total_steal_count",
    "max_steal_count",
    "min_steal_count",
    "remote_schedules",
    "total_local_sched_count",
    "max_local_sched_count",
    "min_local_sched_count",
    "total_overflow_count",
    "max_overflow_count",
    "min_overflow_count",
    "total_polls_count",
    "max_polls_count",
    "min_polls_count",
    "total_busy_duration",
    "max_busy_duration",
    "min_busy_duration",
    "injection_queue_depth",
    "total_local_queue_depth",
    "max_local_queue_depth",
    "min_local_queue_depth",
    "elapsed",
];

static TOKIO_RT_METRICS_DESC: &[&str] = &[
    "Tokio workers count",
    "Tokio total park count",
    "Tokio maximum park count",
    "Tokio minimum park count",
    "Tokio noop count",
    "Tokio maximum noop count",
    "Tokio minimum noop count",
    "Tokio total steal count",
    "Tokio maximum steal count",
    "Tokio minimum steal count",
    "Tokio number of remote schedules",
    "Tokio total local schedules count",
    "Tokio maximum local schedules count",
    "Tokio minimum local schedules count",
    "Tokio total overflow count",
    "Tokio maximum overflow count",
    "Tokio minimum overflow count",
    "Tokio total polls count",
    "Tokio maximum polls count",
    "Tokio minimum polls count",
    "Tokio total busy duration",
    "Tokio maximum busy duration",
    "Tokio minimum busy duration",
    "Tokio injection queue depth",
    "Tokio total local queue depth",
    "Tokio maximum local queue depth",
    "Tokio minimum local queue depth",
    "Tokio elapsed time",
];

pub struct TokioRuntimeMetrics {
    pub metric_gauges: HashMap<String, Gauge>,
}

impl TokioRuntimeMetrics {
    pub fn new() -> Self {
        TokioRuntimeMetrics {
            metric_gauges: HashMap::new(),
        }
    }
    pub fn register(&mut self, registry: &mut Registry) {
        let sub_registry = registry.sub_registry_with_prefix("tokio");

        for i in 0..TOKIO_RT_METRICS_NAME.len() {
            let gauge: Gauge = Gauge::default();
            sub_registry.register(
                TOKIO_RT_METRICS_NAME[i],
                TOKIO_RT_METRICS_DESC[i],
                Box::new(gauge.clone()),
            );
            self.metric_gauges
                .insert(TOKIO_RT_METRICS_NAME[i].to_string(), gauge);
        }
    }

    pub async fn update_metric_values(
        tokio_rt_metrics: Arc<RwLock<Self>>,
        runtime_monitor: RuntimeMonitor,
    ) {
        let tokio_rt_metrics = tokio_rt_metrics.clone();
        let mut interval = tokio::time::interval(Duration::from_secs(TOKIO_METRICS_FREQ_SECS));
        let mut runtime_intervals = runtime_monitor.intervals();

        loop {
            interval.tick().await;
            if let Some(interval) = runtime_intervals.next() {
                tokio_rt_metrics.write().update_tokio_metrics(interval);
            }
        }
    }

    pub fn update_metric_value(&mut self, name: &str, value: u64) {
        if let Some(gauge) = self.metric_gauges.get(&name.to_string()) {
            gauge.set(value);
        } else {
            panic!("Unexpected metric name: {}", name);
        }
    }

    pub fn update_tokio_metrics(&mut self, interval: RuntimeMetrics) {
        self.update_metric_value(TOKIO_RT_METRICS_NAME[0], interval.workers_count as u64);
        self.update_metric_value(TOKIO_RT_METRICS_NAME[1], interval.total_park_count);
        self.update_metric_value(TOKIO_RT_METRICS_NAME[2], interval.max_park_count);
        self.update_metric_value(TOKIO_RT_METRICS_NAME[3], interval.min_park_count);
        self.update_metric_value(TOKIO_RT_METRICS_NAME[4], interval.total_noop_count);
        self.update_metric_value(TOKIO_RT_METRICS_NAME[5], interval.max_noop_count);
        self.update_metric_value(TOKIO_RT_METRICS_NAME[6], interval.min_noop_count);
        self.update_metric_value(TOKIO_RT_METRICS_NAME[7], interval.total_steal_count);
        self.update_metric_value(TOKIO_RT_METRICS_NAME[8], interval.max_steal_count);
        self.update_metric_value(TOKIO_RT_METRICS_NAME[9], interval.min_steal_count);
        self.update_metric_value(TOKIO_RT_METRICS_NAME[10], interval.num_remote_schedules);
        self.update_metric_value(
            TOKIO_RT_METRICS_NAME[11],
            interval.total_local_schedule_count,
        );
        self.update_metric_value(TOKIO_RT_METRICS_NAME[12], interval.max_local_schedule_count);
        self.update_metric_value(TOKIO_RT_METRICS_NAME[13], interval.min_local_schedule_count);
        self.update_metric_value(TOKIO_RT_METRICS_NAME[14], interval.total_overflow_count);
        self.update_metric_value(TOKIO_RT_METRICS_NAME[15], interval.max_overflow_count);
        self.update_metric_value(TOKIO_RT_METRICS_NAME[16], interval.min_overflow_count);
        self.update_metric_value(TOKIO_RT_METRICS_NAME[17], interval.total_polls_count);
        self.update_metric_value(TOKIO_RT_METRICS_NAME[18], interval.max_polls_count);
        self.update_metric_value(TOKIO_RT_METRICS_NAME[19], interval.min_polls_count);
        self.update_metric_value(
            TOKIO_RT_METRICS_NAME[20],
            interval.total_busy_duration.as_millis() as u64,
        );
        self.update_metric_value(
            TOKIO_RT_METRICS_NAME[21],
            interval.max_busy_duration.as_millis() as u64,
        );
        self.update_metric_value(
            TOKIO_RT_METRICS_NAME[22],
            interval.min_busy_duration.as_millis() as u64,
        );
        self.update_metric_value(
            TOKIO_RT_METRICS_NAME[23],
            interval.injection_queue_depth as u64,
        );
        self.update_metric_value(
            TOKIO_RT_METRICS_NAME[24],
            interval.total_local_queue_depth as u64,
        );
        self.update_metric_value(
            TOKIO_RT_METRICS_NAME[25],
            interval.max_local_queue_depth as u64,
        );
        self.update_metric_value(
            TOKIO_RT_METRICS_NAME[26],
            interval.min_local_queue_depth as u64,
        );
        self.update_metric_value(
            TOKIO_RT_METRICS_NAME[27],
            interval.elapsed.as_micros() as u64,
        );
    }
}
