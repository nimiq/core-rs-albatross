use std::io;
use std::sync::Arc;

use blockchain::Blockchain;
use block::Difficulty;

use crate::server;
use crate::server::SerializationType;

pub struct ChainMetrics {
    blockchain: Arc<Blockchain<'static>>,
}

impl ChainMetrics {
    pub fn new(blockchain: Arc<Blockchain<'static>>) -> Self {
        ChainMetrics {
            blockchain,
        }
    }
}

impl server::Metrics for ChainMetrics {
    fn metrics(&self, serializer: &mut server::MetricsSerializer<SerializationType>) -> Result<(), io::Error> {
        // Release lock as fast as possible.
        {
            let head = self.blockchain.head();

            serializer.metric("chain_head_height", head.header.height)?;
            serializer.metric("chain_head_difficulty", Difficulty::from(head.header.n_bits))?;
            serializer.metric("chain_head_transactions", head.body.as_ref().map(|body| body.transactions.len()).unwrap_or(0))?;
        }
        serializer.metric("chain_total_work", self.blockchain.total_work().clone())?;

        serializer.metric_with_attributes("chain_block", self.blockchain.metrics.block_forked_count(), attributes!{"action" => "forked"})?;
        serializer.metric_with_attributes("chain_block", self.blockchain.metrics.block_rebranched_count(), attributes!{"action" => "rebranched"})?;
        serializer.metric_with_attributes("chain_block", self.blockchain.metrics.block_extended_count(), attributes!{"action" => "extended"})?;
        serializer.metric_with_attributes("chain_block", self.blockchain.metrics.block_orphan_count(), attributes!{"action" => "orphan"})?;
        serializer.metric_with_attributes("chain_block", self.blockchain.metrics.block_invalid_count(), attributes!{"action" => "invalid"})?;
        serializer.metric_with_attributes("chain_block", self.blockchain.metrics.block_known_count(), attributes!{"action" => "known"})?;

        Ok(())
    }
}