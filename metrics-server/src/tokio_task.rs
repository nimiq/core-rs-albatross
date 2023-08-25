use std::{collections::HashMap, sync::Arc, time::Duration};

use parking_lot::RwLock;
use prometheus_client::{metrics::gauge::Gauge, registry::Registry};
use tokio_metrics::{TaskMetrics, TaskMonitor};

const TOKIO_METRICS_FREQ_SECS: u64 = 1;

static TOKIO_TASK_METRICS_NAME: &[&str] = &[
    "instrumented_count",
    "dropped_count",
    "first_poll_count",
    "total_first_poll_delay",
    "total_idled_count",
    "total_idle_duration",
    "total_schedule_count",
    "total_schedule_duration",
    "total_poll_count",
    "total_poll_duration",
    "total_fast_poll_count",
    "total_fast_poll_duration",
    "total_slow_poll_count",
    "total_slow_poll_duration",
    "total_short_delay_count",
    "total_long_delay_count",
    "total_short_delay_duration",
    "total_long_delay_duration",
];

static TOKIO_TASK_METRICS_DESC: &[&str] = &[
    "number of tasks instrumented",
    "number of tasks dropped",
    "number of tasks polled for the first time",
    "total duration elapsed between the instant tasks are instrumented, and the instant they are first polled",
    "total number of times that tasks idled, waiting to be awoken",
    "total duration that tasks idled",
    "total number of times that tasks were awoken (and then, presumably, scheduled for execution)",
    "total duration that tasks spent waiting to be polled after awakening",
    "total number of times that tasks were polled",
    "total duration elapsed during polls",
    "total number of times that polling tasks completed swiftly",
    "total duration of fast polls",
    "total number of times that polling tasks completed slowly",
    "total duration of slow polls",
    "total count of tasks with short scheduling delay",
    "total count of tasks with long scheduling delays",
    "total duration of tasks with short scheduling delays",
    "total duration of tasks with long scheduling delays",
];

pub struct TokioTaskMetrics {
    pub metric_gauges: HashMap<String, Gauge>,
}

impl TokioTaskMetrics {
    pub fn new() -> Self {
        TokioTaskMetrics {
            metric_gauges: HashMap::new(),
        }
    }
    pub fn register(&mut self, registry: &mut Registry, task_names: &[String]) {
        let sub_registry = registry.sub_registry_with_prefix("tokio");

        for task_name in task_names {
            for i in 0..TOKIO_TASK_METRICS_NAME.len() {
                let gauge: Gauge = Gauge::default();
                sub_registry.register(
                    format!("{}_{}", task_name, TOKIO_TASK_METRICS_NAME[i]),
                    format!("{} {}", task_name, TOKIO_TASK_METRICS_DESC[i]),
                    gauge.clone(),
                );
                self.metric_gauges.insert(
                    format!("{}_{}", task_name, TOKIO_TASK_METRICS_NAME[i]),
                    gauge,
                );
            }
        }
    }

    pub async fn update_metric_values(
        tokio_task_metrics: Arc<RwLock<Self>>,
        task_name: &str,
        runtime_monitor: TaskMonitor,
    ) {
        let tokio_task_metrics = tokio_task_metrics.clone();
        let mut interval = tokio::time::interval(Duration::from_secs(TOKIO_METRICS_FREQ_SECS));
        let mut runtime_intervals = runtime_monitor.intervals();

        loop {
            interval.tick().await;
            if let Some(interval) = runtime_intervals.next() {
                tokio_task_metrics
                    .write()
                    .update_tokio_task_metrics(task_name, interval);
            }
        }
    }

    pub fn update_metric_value(&mut self, name: String, value: u64) {
        if let Some(gauge) = self.metric_gauges.get(&name) {
            gauge.set(value as i64);
        } else {
            panic!("Unexpected metric name: {name}");
        }
    }

    pub fn update_tokio_task_metrics(&mut self, task_name: &str, interval: TaskMetrics) {
        self.update_metric_value(
            format!("{}_{}", task_name, TOKIO_TASK_METRICS_NAME[0]),
            interval.instrumented_count,
        );
        self.update_metric_value(
            format!("{}_{}", task_name, TOKIO_TASK_METRICS_NAME[1]),
            interval.dropped_count,
        );
        self.update_metric_value(
            format!("{}_{}", task_name, TOKIO_TASK_METRICS_NAME[2]),
            interval.first_poll_count,
        );
        self.update_metric_value(
            format!("{}_{}", task_name, TOKIO_TASK_METRICS_NAME[3]),
            interval.total_first_poll_delay.as_micros() as u64,
        );
        self.update_metric_value(
            format!("{}_{}", task_name, TOKIO_TASK_METRICS_NAME[4]),
            interval.total_idled_count,
        );
        self.update_metric_value(
            format!("{}_{}", task_name, TOKIO_TASK_METRICS_NAME[5]),
            interval.total_idle_duration.as_micros() as u64,
        );
        self.update_metric_value(
            format!("{}_{}", task_name, TOKIO_TASK_METRICS_NAME[6]),
            interval.total_scheduled_count,
        );
        self.update_metric_value(
            format!("{}_{}", task_name, TOKIO_TASK_METRICS_NAME[7]),
            interval.total_scheduled_duration.as_micros() as u64,
        );
        self.update_metric_value(
            format!("{}_{}", task_name, TOKIO_TASK_METRICS_NAME[8]),
            interval.total_poll_count,
        );
        self.update_metric_value(
            format!("{}_{}", task_name, TOKIO_TASK_METRICS_NAME[9]),
            interval.total_poll_duration.as_micros() as u64,
        );
        self.update_metric_value(
            format!("{}_{}", task_name, TOKIO_TASK_METRICS_NAME[10]),
            interval.total_fast_poll_count,
        );
        self.update_metric_value(
            format!("{}_{}", task_name, TOKIO_TASK_METRICS_NAME[11]),
            interval.total_fast_poll_duration.as_micros() as u64,
        );
        self.update_metric_value(
            format!("{}_{}", task_name, TOKIO_TASK_METRICS_NAME[12]),
            interval.total_slow_poll_count,
        );
        self.update_metric_value(
            format!("{}_{}", task_name, TOKIO_TASK_METRICS_NAME[13]),
            interval.total_slow_poll_duration.as_micros() as u64,
        );
        self.update_metric_value(
            format!("{}_{}", task_name, TOKIO_TASK_METRICS_NAME[14]),
            interval.total_short_delay_count,
        );
        self.update_metric_value(
            format!("{}_{}", task_name, TOKIO_TASK_METRICS_NAME[15]),
            interval.total_long_delay_count,
        );
        self.update_metric_value(
            format!("{}_{}", task_name, TOKIO_TASK_METRICS_NAME[16]),
            interval.total_short_delay_duration.as_micros() as u64,
        );
        self.update_metric_value(
            format!("{}_{}", task_name, TOKIO_TASK_METRICS_NAME[17]),
            interval.total_long_delay_duration.as_micros() as u64,
        );
    }
}
