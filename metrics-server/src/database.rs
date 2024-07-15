use prometheus_client::registry::Registry;

use crate::metrics::MetricsCollector;

pub struct DatabaseMetrics {}

impl DatabaseMetrics {
    pub fn register(registry: &mut Registry, collector: MetricsCollector) {
        let sub_registry = registry.sub_registry_with_prefix("database");
        sub_registry.register_collector(Box::new(collector));
    }
}
