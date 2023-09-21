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
    "mean_poll_duration",
    "mean_poll_duration_worker_min",
    "mean_poll_duration_worker_max",
    "total_noop_count",
    "max_noop_count",
    "min_noop_count",
    "total_steal_count",
    "max_steal_count",
    "min_steal_count",
    "total_steal_operations",
    "max_steal_operations",
    "min_steal_operations",
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
    "budget_forced_yield_count",
    "io_driver_ready_count",
];

static TOKIO_RT_METRICS_DESC: &[&str] = &[
    "Tokio number of worker threads used by the runtime",
    "Tokio number of times worker threads parked",
    "Tokio maximum number of times any worker thread parked",
    "Tokio minimum number of times any worker thread parked",
    "Tokio average duration of a single invocation of poll on a task",
    "Tokio average duration of a single invocation of poll on a task on the worker with the lowest value",
    "Tokio average duration of a single invocation of poll on a task on the worker with the highest value",
    "Tokio number of times worker threads unparked but performed no work before parking again",
    "Tokio maximum number of times any worker thread unparked but performed no work before parking again",
    "Tokio minimum number of times any worker thread unparked but performed no work before parking again",
    "Tokio number of tasks worker threads stole from another worker thread",
    "Tokio maximum number of tasks any worker thread stole from another worker thread",
    "Tokio minimum number of tasks any worker thread stole from another worker thread",
    "Tokio number of times worker threads stole tasks from another worker thread",
    "Tokio maximum number of times any worker thread stole tasks from another worker thread",
    "Tokio minimum number of times any worker thread stole tasks from another worker thread",
    "Tokio number of tasks scheduled from outside of the runtime",
    "Tokio number of tasks scheduled from worker threads",
    "Tokio maximum number of tasks scheduled from any one worker thread",
    "Tokio minimum number of tasks scheduled from any one worker thread",
    "Tokio number of times worker threads saturated their local queues",
    "Tokio maximum number of times any one worker saturated its local queue",
    "Tokio minimum number of times any one worker saturated its local queue",
    "Tokio number of tasks that have been polled across all worker threads",
    "Tokio maximum number of tasks that have been polled in any worker thread",
    "Tokio minimum number of tasks that have been polled in any worker thread",
    "Tokio amount of time worker threads were busy",
    "Tokio maximum amount of time a worker thread was busy",
    "Tokio minimum amount of time a worker thread was busy",
    "Tokio number of tasks currently scheduled in the runtime's injection queue",
    "Tokio total number of tasks currently scheduled in workers' local queues",
    "Tokio maximum number of tasks currently scheduled any worker's local queue",
    "Tokio minimum number of tasks currently scheduled any worker's local queue",
    "Tokio total amount of time elapsed since observing runtime metrics",
    "Tokio number of times that tasks have been forced to yield back to the scheduler after exhausting their task budgets",
    "Tokio number of ready events processed by the runtime's I/O driver",
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
                gauge.clone(),
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
            gauge.set(value as i64);
        } else {
            panic!("Unexpected metric name: {}", name);
        }
    }

    pub fn update_tokio_metrics(&mut self, interval: RuntimeMetrics) {
        self.update_metric_value(TOKIO_RT_METRICS_NAME[0], interval.workers_count as u64);
        self.update_metric_value(TOKIO_RT_METRICS_NAME[1], interval.total_park_count);
        self.update_metric_value(TOKIO_RT_METRICS_NAME[2], interval.max_park_count);
        self.update_metric_value(TOKIO_RT_METRICS_NAME[3], interval.min_park_count);
        self.update_metric_value(
            TOKIO_RT_METRICS_NAME[4],
            interval.mean_poll_duration.as_micros() as u64,
        );
        self.update_metric_value(
            TOKIO_RT_METRICS_NAME[5],
            interval.mean_poll_duration_worker_min.as_micros() as u64,
        );
        self.update_metric_value(
            TOKIO_RT_METRICS_NAME[6],
            interval.mean_poll_duration_worker_max.as_micros() as u64,
        );
        self.update_metric_value(TOKIO_RT_METRICS_NAME[7], interval.total_noop_count);
        self.update_metric_value(TOKIO_RT_METRICS_NAME[8], interval.max_noop_count);
        self.update_metric_value(TOKIO_RT_METRICS_NAME[9], interval.min_noop_count);
        self.update_metric_value(TOKIO_RT_METRICS_NAME[10], interval.total_steal_count);
        self.update_metric_value(TOKIO_RT_METRICS_NAME[11], interval.max_steal_count);
        self.update_metric_value(TOKIO_RT_METRICS_NAME[12], interval.min_steal_count);
        self.update_metric_value(TOKIO_RT_METRICS_NAME[13], interval.total_steal_operations);
        self.update_metric_value(TOKIO_RT_METRICS_NAME[14], interval.max_steal_operations);
        self.update_metric_value(TOKIO_RT_METRICS_NAME[15], interval.min_steal_operations);
        self.update_metric_value(TOKIO_RT_METRICS_NAME[16], interval.num_remote_schedules);
        self.update_metric_value(
            TOKIO_RT_METRICS_NAME[17],
            interval.total_local_schedule_count,
        );
        self.update_metric_value(TOKIO_RT_METRICS_NAME[18], interval.max_local_schedule_count);
        self.update_metric_value(TOKIO_RT_METRICS_NAME[19], interval.min_local_schedule_count);
        self.update_metric_value(TOKIO_RT_METRICS_NAME[20], interval.total_overflow_count);
        self.update_metric_value(TOKIO_RT_METRICS_NAME[21], interval.max_overflow_count);
        self.update_metric_value(TOKIO_RT_METRICS_NAME[22], interval.min_overflow_count);
        self.update_metric_value(TOKIO_RT_METRICS_NAME[23], interval.total_polls_count);
        self.update_metric_value(TOKIO_RT_METRICS_NAME[24], interval.max_polls_count);
        self.update_metric_value(TOKIO_RT_METRICS_NAME[25], interval.min_polls_count);
        self.update_metric_value(
            TOKIO_RT_METRICS_NAME[26],
            interval.total_busy_duration.as_millis() as u64,
        );
        self.update_metric_value(
            TOKIO_RT_METRICS_NAME[27],
            interval.max_busy_duration.as_millis() as u64,
        );
        self.update_metric_value(
            TOKIO_RT_METRICS_NAME[28],
            interval.min_busy_duration.as_millis() as u64,
        );
        self.update_metric_value(
            TOKIO_RT_METRICS_NAME[29],
            interval.injection_queue_depth as u64,
        );
        self.update_metric_value(
            TOKIO_RT_METRICS_NAME[30],
            interval.total_local_queue_depth as u64,
        );
        self.update_metric_value(
            TOKIO_RT_METRICS_NAME[31],
            interval.max_local_queue_depth as u64,
        );
        self.update_metric_value(
            TOKIO_RT_METRICS_NAME[32],
            interval.min_local_queue_depth as u64,
        );
        self.update_metric_value(
            TOKIO_RT_METRICS_NAME[33],
            interval.elapsed.as_micros() as u64,
        );
        self.update_metric_value(
            TOKIO_RT_METRICS_NAME[34],
            interval.budget_forced_yield_count,
        );
        self.update_metric_value(TOKIO_RT_METRICS_NAME[35], interval.io_driver_ready_count);
    }
}
