use std::io;
use std::sync::Arc;

use block::Difficulty;
use blockchain::Blockchain;
use blockchain_albatross::Blockchain as AlbatrossBlockchain;
use blockchain_base::AbstractBlockchain;
use consensus::{AlbatrossConsensusProtocol, ConsensusProtocol, NimiqConsensusProtocol};

use crate::server;
use crate::server::{Metrics, SerializationType};

pub trait AbstractChainMetrics<P: ConsensusProtocol + 'static> {
    fn new(blockchain: Arc<P::Blockchain>) -> Self;

    //fn metrics(&self, serializer: &mut server::MetricsSerializer<SerializationType>) -> Result<(), io::Error>;

    fn serialize_blockchain_metrics(
        &self,
        blockchain: Arc<P::Blockchain>,
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

pub struct NimiqChainMetrics {
    blockchain: Arc<Blockchain>,
}

impl AbstractChainMetrics<NimiqConsensusProtocol> for NimiqChainMetrics {
    fn new(blockchain: Arc<Blockchain>) -> Self {
        Self { blockchain }
    }
}

impl Metrics for NimiqChainMetrics {
    fn metrics(
        &self,
        serializer: &mut server::MetricsSerializer<SerializationType>,
    ) -> Result<(), io::Error> {
        // Release lock as fast as possible.
        {
            let head = self.blockchain.head();

            serializer.metric("chain_head_height", head.header.height)?;
            serializer.metric(
                "chain_head_difficulty",
                Difficulty::from(head.header.n_bits),
            )?;
            serializer.metric(
                "chain_head_transactions",
                head.body
                    .as_ref()
                    .map(|body| body.transactions.len())
                    .unwrap_or(0),
            )?;
        }
        serializer.metric("chain_total_work", self.blockchain.total_work())?;

        self.serialize_blockchain_metrics(Arc::clone(&self.blockchain), serializer)?;

        Ok(())
    }
}

pub struct AlbatrossChainMetrics {
    blockchain: Arc<AlbatrossBlockchain>,
}

impl AbstractChainMetrics<AlbatrossConsensusProtocol> for AlbatrossChainMetrics {
    fn new(blockchain: Arc<AlbatrossBlockchain>) -> Self {
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
