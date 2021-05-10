use std::io;
use std::sync::Arc;

use blockchain::{AbstractBlockchain, Blockchain};

use crate::server;
use crate::server::{Metrics, SerializationType};

pub trait AbstractChainMetrics {
    fn new(blockchain: Arc<Blockchain>) -> Self;

    //fn metrics(&self, serializer: &mut server::MetricsSerializer<SerializationType>) -> Result<(), io::Error>;

    fn serialize_blockchain_metrics(
        &self,
        blockchain: Arc<Blockchain>,
        serializer: &mut server::MetricsSerializer<SerializationType>,
    ) -> Result<(), io::Error> {
        let metrics = blockchain.metrics();
        serializer.metric_with_attributes(
            "chain_block",
            metrics.block_forked_count(),
            attributes! {"action" => "forked"},
        )?;
        serializer.metric_with_attributes(
            "chain_block",
            metrics.block_rebranched_count(),
            attributes! {"action" => "rebranched"},
        )?;
        serializer.metric_with_attributes(
            "chain_block",
            metrics.block_extended_count(),
            attributes! {"action" => "extended"},
        )?;
        serializer.metric_with_attributes(
            "chain_block",
            metrics.block_orphan_count(),
            attributes! {"action" => "orphan"},
        )?;
        serializer.metric_with_attributes(
            "chain_block",
            metrics.block_invalid_count(),
            attributes! {"action" => "invalid"},
        )?;
        serializer.metric_with_attributes(
            "chain_block",
            metrics.block_known_count(),
            attributes! {"action" => "known"},
        )?;
        Ok(())
    }
}

pub struct AlbatrossChainMetrics {
    blockchain: Arc<Blockchain>,
}

impl AbstractChainMetrics for AlbatrossChainMetrics {
    fn new(blockchain: Arc<Blockchain>) -> Self {
        Self { blockchain }
    }
}

impl Metrics for AlbatrossChainMetrics {
    fn metrics(
        &self,
        serializer: &mut server::MetricsSerializer<SerializationType>,
    ) -> Result<(), io::Error> {
        // Release lock as fast as possible.
        {
            let head = self.blockchain.head();

            serializer.metric("chain_head_height", head.block_number())?;
            serializer.metric("chain_head_view_number", head.view_number())?;

            /*match head {
                AlbatrossBlock::Macro(ref macro_block) => {
                    // TODO metrics about macro block?
                    // e.g.: number of signatures in prepare & commit
                }
                AlbatrossBlock::Micro(ref micro_block) => {
                    // TODO: number of transactions in micro block
                    // other metrics?
                }
            }*/
        }

        self.serialize_blockchain_metrics(Arc::clone(&self.blockchain), serializer)?;

        Ok(())
    }
}
