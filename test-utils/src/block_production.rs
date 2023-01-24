use std::sync::Arc;

use parking_lot::RwLock;

use beserial::Deserialize;
use nimiq_block::{
    Block, MacroBlock, MacroBody, MultiSignature, SignedSkipBlockInfo, SkipBlockInfo,
    SkipBlockProof, TendermintIdentifier, TendermintProof, TendermintProposal, TendermintStep,
    TendermintVote,
};
use nimiq_block_production::BlockProducer;
use nimiq_blockchain::{Blockchain, BlockchainConfig, ChunksPushError, ChunksPushResult};
use nimiq_blockchain_interface::{AbstractBlockchain, PushError, PushResult};
use nimiq_bls::{AggregateSignature, KeyPair as BlsKeyPair, SecretKey as BlsSecretKey};
use nimiq_collections::BitSet;
use nimiq_database::volatile::VolatileEnvironment;
use nimiq_genesis::NetworkId;
use nimiq_hash::Blake2sHash;
use nimiq_keys::{KeyPair as SchnorrKeyPair, PrivateKey as SchnorrPrivateKey};
use nimiq_primitives::{
    key_nibbles::KeyNibbles, policy::Policy, trie::trie_chunk::TrieChunkWithStart,
};
use nimiq_transaction::Transaction;
use nimiq_utils::time::OffsetTime;

/// Secret keys of validator. Tests run with `genesis/src/genesis/unit-albatross.toml`
const SIGNING_KEY: &str = "041580cc67e66e9e08b68fd9e4c9deb68737168fbe7488de2638c2e906c2f5ad";
const VOTING_KEY: &str = "196ffdb1a8acc7cbd76a251aeac0600a1d68b3aba1eba823b5e4dc5dbdcdc730afa752c05ab4f6ef8518384ad514f403c5a088a22b17bf1bc14f8ff8decc2a512c0a200f68d7bdf5a319b30356fe8d1d75ef510aed7a8660968c216c328a0000";

pub struct TemporaryBlockProducer {
    pub blockchain: Arc<RwLock<Blockchain>>,
    pub producer: BlockProducer,
}

impl Default for TemporaryBlockProducer {
    fn default() -> Self {
        Self::new()
    }
}

impl TemporaryBlockProducer {
    pub fn new_incomplete() -> Self {
        let producer = Self::new();

        // Reset accounts trie.
        {
            let blockchain_rg = producer.blockchain.read();
            let mut txn = blockchain_rg.write_transaction();
            blockchain_rg
                .state
                .accounts
                .reinitialize_as_incomplete(&mut txn);
            txn.commit();
        }

        producer
    }

    pub fn new() -> Self {
        let time = Arc::new(OffsetTime::new());
        let env = VolatileEnvironment::new(10).unwrap();
        let blockchain = Arc::new(RwLock::new(
            Blockchain::new(
                env,
                BlockchainConfig::default(),
                NetworkId::UnitAlbatross,
                time,
            )
            .unwrap(),
        ));

        let signing_key = SchnorrKeyPair::from(
            SchnorrPrivateKey::deserialize_from_vec(&hex::decode(SIGNING_KEY).unwrap()).unwrap(),
        );
        let voting_key = BlsKeyPair::from(
            BlsSecretKey::deserialize_from_vec(&hex::decode(VOTING_KEY).unwrap()).unwrap(),
        );
        let producer: BlockProducer = BlockProducer::new(signing_key, voting_key);
        TemporaryBlockProducer {
            blockchain,
            producer,
        }
    }

    pub fn push(&self, block: Block) -> Result<PushResult, PushError> {
        Blockchain::push(self.blockchain.upgradable_read(), block)
    }

    pub fn push_with_chunks(
        &self,
        block: Block,
        chunks: Vec<TrieChunkWithStart>,
    ) -> Result<(PushResult, Result<ChunksPushResult, ChunksPushError>), PushError> {
        Blockchain::push_with_chunks(self.blockchain.upgradable_read(), block, chunks)
    }

    pub fn get_chunk(&self, start_key: KeyNibbles, limit: usize) -> TrieChunkWithStart {
        let chunk = self
            .blockchain
            .read()
            .state
            .accounts
            .get_chunk(start_key.clone(), limit, None);
        TrieChunkWithStart { chunk, start_key }
    }

    pub fn next_block(&self, extra_data: Vec<u8>, skip_block: bool) -> Block {
        self.next_block_with_txs(extra_data, skip_block, vec![])
    }

    pub fn next_block_with_txs(
        &self,
        extra_data: Vec<u8>,
        skip_block: bool,
        transactions: Vec<Transaction>,
    ) -> Block {
        let block = self.next_block_no_push_with_txs(extra_data, skip_block, transactions);
        assert_eq!(self.push(block.clone()), Ok(PushResult::Extended));
        block
    }

    pub fn next_block_no_push(&self, extra_data: Vec<u8>, skip_block: bool) -> Block {
        self.next_block_no_push_with_txs(extra_data, skip_block, vec![])
    }

    pub fn next_block_no_push_with_txs(
        &self,
        extra_data: Vec<u8>,
        skip_block: bool,
        transactions: Vec<Transaction>,
    ) -> Block {
        let blockchain = self.blockchain.read();

        let height = blockchain.block_number() + 1;

        let block = if Policy::is_macro_block_at(height) {
            let macro_block_proposal = self.producer.next_macro_block_proposal(
                &blockchain,
                blockchain.head().timestamp() + Policy::BLOCK_SEPARATION_TIME,
                0,
                extra_data,
            );

            // Calculate the block hash.
            let block_hash = macro_block_proposal.nano_zkp_hash(true);

            // Get validator set and make sure it exists.
            let validators = blockchain
                .get_validators_for_epoch(Policy::epoch_at(blockchain.block_number() + 1), None);
            assert!(validators.is_ok());

            Block::Macro(TemporaryBlockProducer::finalize_macro_block(
                TendermintProposal {
                    valid_round: None,
                    value: macro_block_proposal.header,
                    round: 0u32,
                },
                macro_block_proposal
                    .body
                    .or_else(|| Some(MacroBody::default()))
                    .unwrap(),
                block_hash,
            ))
        } else if skip_block {
            Block::Micro(self.producer.next_micro_block(
                &blockchain,
                blockchain.head().timestamp() + Policy::BLOCK_PRODUCER_TIMEOUT,
                vec![],
                vec![],
                extra_data,
                Some(self.create_skip_block_proof()),
            ))
        } else {
            Block::Micro(self.producer.next_micro_block(
                &blockchain,
                blockchain.head().timestamp() + Policy::BLOCK_SEPARATION_TIME,
                vec![],
                transactions,
                extra_data,
                None,
            ))
        };

        // drop the lock before pushing the block as that will acquire write eventually
        drop(blockchain);

        block
    }

    pub fn finalize_macro_block(
        proposal: TendermintProposal,
        body: MacroBody,
        block_hash: Blake2sHash,
    ) -> MacroBlock {
        let keypair = BlsKeyPair::from(
            BlsSecretKey::deserialize_from_vec(&hex::decode(VOTING_KEY).unwrap()).unwrap(),
        );

        // Create a TendermintVote instance out of known properties.
        // round_number is for now fixed at 0 for tests, but it could be anything,
        // as long as the TendermintProof further down this function does use the same round_number.
        let vote = TendermintVote {
            proposal_hash: Some(block_hash),
            id: TendermintIdentifier {
                block_number: proposal.value.block_number,
                step: TendermintStep::PreCommit,
                round_number: 0,
            },
        };

        // sign the hash
        let signature = AggregateSignature::from_signatures(&[keypair
            .secret_key
            .sign(&vote)
            .multiply(Policy::SLOTS)]);

        // create and populate signers BitSet.
        let mut signers = BitSet::new();
        for i in 0..Policy::SLOTS {
            signers.insert(i as usize);
        }

        // create the TendermintProof
        let justification = Some(TendermintProof {
            round: 0,
            sig: MultiSignature::new(signature, signers),
        });

        MacroBlock {
            header: proposal.value,
            justification,
            body: Some(body),
        }
    }

    pub fn create_skip_block_proof(&self) -> SkipBlockProof {
        let keypair = BlsKeyPair::from(
            BlsSecretKey::deserialize_from_vec(&hex::decode(VOTING_KEY).unwrap()).unwrap(),
        );

        let skip_block_info = {
            let blockchain = self.blockchain.read();
            SkipBlockInfo {
                block_number: blockchain.block_number() + 1,
                vrf_entropy: blockchain.head().seed().entropy(),
            }
        };

        // create signed skip block information
        let skip_block_info =
            SignedSkipBlockInfo::from_message(skip_block_info, &keypair.secret_key, 0);

        let signature = AggregateSignature::from_signatures(&[skip_block_info
            .signature
            .multiply(Policy::SLOTS)]);
        let mut signers = BitSet::new();
        for i in 0..Policy::SLOTS {
            signers.insert(i as usize);
        }

        // create proof
        SkipBlockProof {
            sig: MultiSignature::new(signature, signers),
        }
    }
}
