use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use bls::bls12_381::{SecretKey, PublicKey, Signature};
use blockchain_albatross::Blockchain;
use block_albatross::{Block, MacroBlock, MicroBlock, BlockType, MacroExtrinsics, MicroExtrinsics};
use block_production_albatross::BlockProducer;


const SECRET_KEY: &'static str = "8049c38d5b20373723dd5fec59a489d65d7ce695be53b1823c4e989cf3458538";


#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MockTimers {
    ProduceBlock
}

pub struct MockValidator<'env> {
    block_producer: Arc<BlockProducer<'env>>,
    timers:  Timers<MockTimers>,
}


impl<'env> MockValidator {
    pub fn new(blockchain: Arc<Blockchain<'env>>, mempool: Arc<Mempool<'env, Blockchain<'env>>>, validator_key: SecretKey) -> Self {
        let validator_key = SecretKey::from_slice(hex::decode(SECRET_KEY).unwrap()).unwrap();
        Self {
            block_producer: Arc::new(BlockProducer::new(blockchain, mempool, validator_key)),
            timers: Timers::new(),
        }
    }

    pub fn start(&self) {
        let block_producer = Arc::clone(&self.block_producer);

        timers.set_interval(MockTimers::ProduceBlock, move || {
            // get next producer
            let (next_producer_idx, next_producer_slot) = block_producer.blockchain.get_next_block_producer();
            // check that we are the producer
            let validator_idx = next_producer_idx; // our own validator index

            // get block type
            let block_type = block_producer.blockchain.get_next_block_type(None);

            // get timestamp
            let timestamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .as_millis() as u64;

            match block_type {
                BlockType::Micro => {
                    let extrinsics = block_producer.blockchain
                        .last_macro_block().extrinsics.clone().unwrap();
                    let block = block_producer.next_macro_header(timestamp, &extrinsics);
                },

                BlockType::Macro => {

                }
            }


        }, Duration::new(5, 0));
    }
}
