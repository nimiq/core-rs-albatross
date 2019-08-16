use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use beserial::Deserialize;
use block_albatross::{Block, BlockType, MacroBlock};
use block_production_albatross::BlockProducer;
use bls::bls12_381::{KeyPair, SecretKey};
use utils::timers::Timers;
use consensus::{Consensus, AlbatrossConsensusProtocol};
use block_albatross::{PbftProofBuilder, PbftPrepareMessage, PbftCommitMessage, SignedPbftPrepareMessage, SignedPbftCommitMessage};
use hash::{Hash, Blake2bHash};
use primitives::policy;


const SECRET_KEY: &str = "49ea68eb6b8afdf4ca4d4c0a0b295c76ca85225293693bc30e755476492b707f";


#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum MockTimers {
    ProduceBlock
}

pub struct MockValidator {
    block_producer: Arc<BlockProducer<'static>>,
    timers: Timers<MockTimers>,
}


impl MockValidator {
    pub fn new(consensus: Arc<Consensus<AlbatrossConsensusProtocol>>) -> Arc<Self> {
        let validator_key = KeyPair::from(SecretKey::deserialize_from_vec(&hex::decode(SECRET_KEY).unwrap()).unwrap());

        Arc::new(Self {
            block_producer: Arc::new(BlockProducer::new(Arc::clone(&consensus.blockchain), Arc::clone(&consensus.mempool), validator_key)),
            timers: Timers::new(),
        })
    }

    pub fn start(&self) {
        info!("Starting mock validator");

        let block_producer = Arc::clone(&self.block_producer);

        self.timers.set_interval(MockTimers::ProduceBlock, move || {
            // get next producer
            // let (next_producer_idx, next_producer_slot) = block_producer.blockchain.get_next_block_producer(0);
            // check that we are the producer
            // let validator_idx = next_producer_idx; // our own validator index

            // get block type
            let block_type = block_producer.blockchain.get_next_block_type(None);

            // get timestamp
            let timestamp = SystemTime::now()
                .duration_since(UNIX_EPOCH).unwrap()
                .as_millis() as u64;

            let block_height = block_producer.blockchain.height();

            let block = match block_type {
                BlockType::Micro => {
                    info!("Producing micro block: {}", block_height + 1);
                    let micro_block = block_producer.next_micro_block(vec![], timestamp, 0, b"Pura Vida!".to_vec(), None);
                    Block::Micro(micro_block)
                },

                BlockType::Macro => {
                    info!("Producing macro block: {}", block_height + 1);

                    let extrinsics = block_producer.blockchain
                        .macro_head().extrinsics.clone().unwrap();
                    let header = block_producer.next_macro_header(timestamp, 0, &extrinsics);
                    let block_hash = header.hash::<Blake2bHash>();

                    // create signed prepare and commit
                    let prepare = SignedPbftPrepareMessage::from_message(
                        PbftPrepareMessage { block_hash: block_hash.clone() },
                        &block_producer.validator_key.secret,
                        0);
                    let commit = SignedPbftCommitMessage::from_message(
                        PbftCommitMessage { block_hash: block_hash.clone() },
                        &block_producer.validator_key.secret,
                        0);

                    // create proof
                    let mut pbft_proof = PbftProofBuilder::new();
                    pbft_proof.add_prepare_signature(
                        &block_producer.validator_key.public,
                        policy::ACTIVE_VALIDATORS,
                        &prepare);
                    pbft_proof.add_commit_signature(
                        &block_producer.validator_key.public,
                        policy::ACTIVE_VALIDATORS,
                        &commit);

                    let macro_block = MacroBlock {
                        header,
                        justification: Some(pbft_proof.build()),
                        extrinsics: Some(extrinsics)
                    };

                    Block::Macro(macro_block)
                }
            };

            let result = block_producer.blockchain.push(block);
            info!("Push result: {:?}", result);

        }, Duration::from_millis(1000));
    }
}
