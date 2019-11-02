use std::io;
use std::sync::Arc;

use beserial::Serialize;
use blockchain_base::AbstractBlockchain;
use mempool::{Mempool, SIZE_MAX};

use crate::server;
use crate::server::SerializationType;

pub struct MempoolMetrics<B: AbstractBlockchain + 'static> {
    mempool: Arc<Mempool<B>>,
}

impl<B: AbstractBlockchain + 'static> MempoolMetrics<B> {
    pub fn new(mempool: Arc<Mempool<B>>) -> Self {
        MempoolMetrics {
            mempool,
        }
    }
}

impl<B: AbstractBlockchain + 'static> server::Metrics for MempoolMetrics<B> {
    fn metrics(&self, serializer: &mut server::MetricsSerializer<SerializationType>) -> Result<(), io::Error> {
        let txs = self.mempool.get_transactions(SIZE_MAX, 0f64);
        let group = [0usize, 1, 2, 5, 10, 20, 50, 100, 200, 500, 1000, 2000, 5000, 10000];
        for i in 1..group.len() {
            let lower_bound = group[i - 1];
            let upper_bound = group[i];
            serializer.metric_with_attributes(
                "mempool_transactions",
                txs.iter().filter(|tx| (tx.fee_per_byte() as usize) >= lower_bound && (tx.fee_per_byte() as usize) < upper_bound).count(),
                attributes!{"fee_per_byte" => format!("<{}", group[i])}
            )?;
        }
        let lower_bound = *group.last().unwrap();
        serializer.metric_with_attributes(
            "mempool_transactions",
            txs.iter().filter(|tx| (tx.fee_per_byte() as usize) >= lower_bound).count(),
            attributes!{"fee_per_byte" => format!(">={}", lower_bound)}
        )?;
        serializer.metric(
            "mempool_size",
            txs.iter().map(|tx| tx.serialized_size()).sum::<usize>(),
        )?;

        Ok(())
    }
}
