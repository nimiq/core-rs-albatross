use std::sync::Arc;

use parking_lot::RwLock;

use beserial::Deserialize;
use nimiq_block::{
    Block, MacroBlock, MacroBody, MultiSignature, SignedViewChange, TendermintIdentifier,
    TendermintProof, TendermintProposal, TendermintStep, TendermintVote, ViewChange,
    ViewChangeProof,
};
use nimiq_blockchain::{AbstractBlockchain, Blockchain, PushError, PushResult};
use nimiq_bls::{AggregateSignature, KeyPair as BlsKeyPair, SecretKey as BlsSecretKey};
use nimiq_collections::BitSet;
use nimiq_database::volatile::VolatileEnvironment;
use nimiq_genesis::NetworkId;
use nimiq_hash::{Blake2bHash, Hash};
use nimiq_keys::{KeyPair as SchnorrKeyPair, PrivateKey as SchnorrPrivateKey};
use nimiq_primitives::policy;
use nimiq_utils::time::OffsetTime;

use crate::BlockProducer;

/// Secret keys of validator. Tests run with `genesis/src/genesis/unit-albatross.toml`
const SIGNING_KEY: &str = "041580cc67e66e9e08b68fd9e4c9deb68737168fbe7488de2638c2e906c2f5ad";
const VOTING_KEY: &str = "196ffdb1a8acc7cbd76a251aeac0600a1d68b3aba1eba823b5e4dc5dbdcdc730afa752c05ab4f6ef8518384ad514f403c5a088a22b17bf1bc14f8ff8decc2a512c0a200f68d7bdf5a319b30356fe8d1d75ef510aed7a8660968c216c328a0000";

pub struct TemporaryBlockProducer {
    pub blockchain: Arc<RwLock<Blockchain>>,
    pub producer: BlockProducer,
}

impl TemporaryBlockProducer {
    pub fn new() -> Self {
        let time = Arc::new(OffsetTime::new());
        let env = VolatileEnvironment::new(10).unwrap();
        let blockchain = Arc::new(RwLock::new(
            Blockchain::new(env, NetworkId::UnitAlbatross, time).unwrap(),
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

    pub fn next_block(&self, view_number: u32, extra_data: Vec<u8>) -> Block {
        let blockchain = self.blockchain.read();

        let height = blockchain.block_number() + 1;

        let block = if policy::is_macro_block_at(height) {
            let macro_block_proposal = self.producer.next_macro_block_proposal(
                &blockchain,
                blockchain.time.now() + height as u64 * 1000,
                0u32,
                extra_data,
            );
            // Get validator set and make sure it exists.
            let validators = blockchain
                .get_validators_for_epoch(policy::epoch_at(blockchain.block_number() + 1));
            assert!(validators.is_some());

            let validator_merkle_root = MacroBlock::create_pk_tree_root(&validators.unwrap());

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
                validator_merkle_root,
            ))
        } else {
            let view_change_proof = if blockchain.next_view_number() == view_number {
                None
            } else {
                Some(self.create_view_change_proof(view_number))
            };

            Block::Micro(self.producer.next_micro_block(
                &blockchain,
                blockchain.time.now() + height as u64 * 1000,
                view_number,
                view_change_proof,
                vec![],
                vec![],
                extra_data,
            ))
        };

        // drop the lock before pushing the block as that will acquire write eventually
        drop(blockchain);

        assert_eq!(self.push(block.clone()), Ok(PushResult::Extended));
        block
    }

    pub fn finalize_macro_block(
        proposal: TendermintProposal,
        extrinsics: MacroBody,
        validator_merkle_root: Vec<u8>,
    ) -> MacroBlock {
        let keypair = BlsKeyPair::from(
            BlsSecretKey::deserialize_from_vec(&hex::decode(VOTING_KEY).unwrap()).unwrap(),
        );

        // Create a TendermintVote instance out of known properties.
        // round_number is for now fixed at 0 for tests, but it could be anything,
        // as long as the TendermintProof further down this function does use the same round_number.
        let vote = TendermintVote {
            proposal_hash: Some(proposal.value.hash::<Blake2bHash>()),
            id: TendermintIdentifier {
                block_number: proposal.value.block_number,
                step: TendermintStep::PreCommit,
                round_number: 0,
            },
            validator_merkle_root,
        };

        // sign the hash
        let signature = AggregateSignature::from_signatures(&[keypair
            .secret_key
            .sign(&vote)
            .multiply(policy::SLOTS)]);

        // create and populate signers BitSet.
        let mut signers = BitSet::new();
        for i in 0..policy::SLOTS {
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
            body: Some(extrinsics),
        }
    }

    pub fn create_view_change_proof(&self, view_number: u32) -> ViewChangeProof {
        let keypair = BlsKeyPair::from(
            BlsSecretKey::deserialize_from_vec(&hex::decode(VOTING_KEY).unwrap()).unwrap(),
        );

        let view_change = {
            let blockchain = self.blockchain.read();
            ViewChange {
                block_number: blockchain.block_number() + 1,
                new_view_number: view_number,
                vrf_entropy: blockchain.head().seed().entropy(),
            }
        };

        // create signed view change
        let view_change = SignedViewChange::from_message(view_change, &keypair.secret_key, 0);

        let signature =
            AggregateSignature::from_signatures(&[view_change.signature.multiply(policy::SLOTS)]);
        let mut signers = BitSet::new();
        for i in 0..policy::SLOTS {
            signers.insert(i as usize);
        }

        // create proof
        ViewChangeProof {
            sig: MultiSignature::new(signature, signers),
        }
    }
}
