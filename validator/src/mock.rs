use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use beserial::Deserialize;
use block_albatross::{
    Block, BlockType,
    MacroBlock, MacroExtrinsics, MacroHeader,
    MicroBlock, MicroExtrinsics, MicroHeader
};
use block_production_albatross::BlockProducer;
use blockchain_albatross::Blockchain;
use bls::bls12_381::{KeyPair, PublicKey, SecretKey, Signature};
use mempool::Mempool;
use utils::timers::Timers;
use consensus::{Consensus, AlbatrossConsensusProtocol};

const SECRET_KEY: &'static str = "8049c38d5b20373723dd5fec59a489d65d7ce695be53b1823c4e989cf3458538";


#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum MockTimers {
    ProduceBlock
}

pub struct MockValidator {
    block_producer: Arc<BlockProducer<'static>>,
    timers:  Timers<MockTimers>,
}


impl MockValidator {
    pub fn new(consensus: Arc<Consensus<AlbatrossConsensusProtocol>>) -> Arc<Self> {
        let validator_key = SecretKey::deserialize_from_vec(&hex::decode(SECRET_KEY).unwrap()).unwrap();
        Arc::new(Self {
            block_producer: Arc::new(BlockProducer::new(Arc::clone(&consensus.blockchain), Arc::clone(&consensus.mempool), validator_key)),
            timers: Timers::new(),
        })
    }

    pub fn start(&self) {
        info!("Starting mock validator");

        let block_producer = Arc::clone(&self.block_producer);

        self.timers.set_interval(MockTimers::ProduceBlock, move || {
            info!("Producing block");

            /*
            // get next producer
            let (next_producer_idx, next_producer_slot) = block_producer.blockchain.get_next_block_producer();
            // check that we are the producer
            let validator_idx = next_producer_idx; // our own validator index

            // get block type
            let block_type = block_producer.blockchain.get_next_block_type(None);

            // get timestamp
            let timestamp = SystemTime::now()
                .duration_since(UNIX_EPOCH).unwrap()
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
            */
        }, Duration::new(5, 0));
    }
}
