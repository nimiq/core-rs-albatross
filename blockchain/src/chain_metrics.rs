use nimiq_block::{Block, BlockBody::Micro};
use nimiq_blockchain_interface::{ChunksPushError, ChunksPushResult, PushError, PushResult};
use nimiq_hash::Blake2bHash;
use prometheus_client::{
    encoding::{EncodeLabelSet, EncodeLabelValue},
    metrics::{counter::Counter, family::Family},
    registry::Registry,
};

#[derive(Default)]
pub struct BlockchainMetrics {
    block_push_counts: Family<PushResultLabels, Counter>,
    transactions_counts: Family<TransactionProcessedLabels, Counter>,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
struct PushResultLabels {
    push_result: BlockPushResult,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelValue)]
enum BlockPushResult {
    Known,
    Extended,
    Rebranched,
    Forked,
    Ignored,
    Orphan,
    Invalid,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
struct TransactionProcessedLabels {
    ty: TransactionProcessed,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelValue)]
enum TransactionProcessed {
    Applied,
    Reverted,
}

impl BlockchainMetrics {
    pub fn register(&self, registry: &mut Registry) {
        registry.register(
            "block_push_counts",
            "Count of block push results",
            self.block_push_counts.clone(),
        );

        registry.register(
            "transaction_counts",
            "Count of transactions applied/reverted",
            self.transactions_counts.clone(),
        );
    }

    #[inline]
    pub fn note_push_result(
        &self,
        push_result: &Result<(PushResult, Result<ChunksPushResult, ChunksPushError>), PushError>,
    ) {
        let push_result = match push_result {
            Ok((PushResult::Known, _)) => BlockPushResult::Known,
            Ok((PushResult::Extended, _)) => BlockPushResult::Extended,
            Ok((PushResult::Rebranched, _)) => BlockPushResult::Rebranched,
            Ok((PushResult::Forked, _)) => BlockPushResult::Forked,
            Ok((PushResult::Ignored, _)) => BlockPushResult::Ignored,
            Err(PushError::Orphan) => BlockPushResult::Orphan,
            Err(_) => {
                self.note_invalid_block();
                return;
            }
        };
        self.block_push_counts
            .get_or_create(&PushResultLabels { push_result })
            .inc();
    }

    #[inline]
    pub fn note_invalid_block(&self) {
        self.block_push_counts
            .get_or_create(&PushResultLabels {
                push_result: BlockPushResult::Invalid,
            })
            .inc();
    }

    #[inline]
    pub fn note_extend(&self, tx_count: usize) {
        self.transactions_counts
            .get_or_create(&TransactionProcessedLabels {
                ty: TransactionProcessed::Applied,
            })
            .inc_by(tx_count as u64);
    }

    #[inline]
    pub fn note_rebranch(
        &self,
        reverted_blocks: &[(Blake2bHash, Block)],
        adopted_blocks: &[(Blake2bHash, Block)],
    ) {
        for (_, micro_block) in reverted_blocks {
            if let Some(Micro(micro_body)) = micro_block.body() {
                self.transactions_counts
                    .get_or_create(&TransactionProcessedLabels {
                        ty: TransactionProcessed::Reverted,
                    })
                    .inc_by(micro_body.transactions.len() as u64);
            }
        }

        for (_, micro_block) in adopted_blocks {
            if let Some(Micro(micro_body)) = micro_block.body() {
                self.transactions_counts
                    .get_or_create(&TransactionProcessedLabels {
                        ty: TransactionProcessed::Applied,
                    })
                    .inc_by(micro_body.transactions.len() as u64);
            }
        }
    }
}
