use std::sync::Arc;

use beserial::Deserialize;
use nimiq_block_albatross::{
    Block, MacroBlock, MacroBody, MacroHeader, MultiSignature, SignedViewChange,
    TendermintIdentifier, TendermintProof, TendermintProposal, TendermintStep, TendermintVote,
    ViewChange, ViewChangeProof,
};
use nimiq_blockchain_albatross::{AbstractBlockchain, Blockchain, PushError, PushResult};
use nimiq_bls::{AggregateSignature, KeyPair, SecretKey};
use nimiq_collections::BitSet;
use nimiq_database::volatile::VolatileEnvironment;
use nimiq_genesis::NetworkId;
use nimiq_hash::{Blake2bHash, Hash};
use nimiq_nano_sync::pk_tree_construct;
use nimiq_primitives::policy;

use crate::BlockProducer;

/// Secret key of validator. Tests run with `genesis/src/genesis/unit-albatross.toml`
const SECRET_KEY: &str = "196ffdb1a8acc7cbd76a251aeac0600a1d68b3aba1eba823b5e4dc5dbdcdc730afa752c05ab4f6ef8518384ad514f403c5a088a22b17bf1bc14f8ff8decc2a512c0a200f68d7bdf5a319b30356fe8d1d75ef510aed7a8660968c216c328a0000";

pub struct TemporaryBlockProducer {
    pub blockchain: Arc<Blockchain>,
    pub producer: BlockProducer,
}

impl TemporaryBlockProducer {
    pub fn new() -> Self {
        let env = VolatileEnvironment::new(10).unwrap();
        let blockchain = Arc::new(Blockchain::new(env, NetworkId::UnitAlbatross).unwrap());

        let keypair = KeyPair::from(
            SecretKey::deserialize_from_vec(&hex::decode(SECRET_KEY).unwrap()).unwrap(),
        );
        let producer = BlockProducer::new_without_mempool(Arc::clone(&blockchain), keypair);
        TemporaryBlockProducer {
            blockchain,
            producer,
        }
    }

    pub fn push(&self, block: Block) -> Result<PushResult, PushError> {
        self.blockchain.push(block)
    }

    pub fn next_block(&self, view_number: u32, extra_data: Vec<u8>) -> Block {
        let height = self.blockchain.block_number() + 1;

        let block = if policy::is_macro_block_at(height) {
            let macro_block_proposal = self.producer.next_macro_block_proposal(
                self.blockchain.time.now() + height as u64 * 1000,
                0u32,
                extra_data,
            );
            // Get validator set and make sure it exists.
            let validators = self
                .blockchain
                .get_validators_for_epoch(policy::epoch_at(self.blockchain.block_number() + 1));
            assert!(validators.is_some());

            let validator_merkle_root = MacroBlock::create_pk_tree_root(&validators.unwrap());

            Block::Macro(TemporaryBlockProducer::finalize_macro_block(
                TendermintProposal {
                    valid_round: None,
                    value: macro_block_proposal.header,
                },
                macro_block_proposal
                    .body
                    .or(Some(MacroBody::new()))
                    .unwrap(),
                validator_merkle_root,
            ))
        } else {
            let view_change_proof = if self.blockchain.next_view_number() == view_number {
                None
            } else {
                Some(self.create_view_change_proof(view_number))
            };

            Block::Micro(self.producer.next_micro_block(
                self.blockchain.time.now() + height as u64 * 1000,
                view_number,
                view_change_proof,
                vec![],
                extra_data,
            ))
        };

        assert_eq!(self.push(block.clone()), Ok(PushResult::Extended));
        block
    }

    pub fn finalize_macro_block(
        proposal: TendermintProposal,
        extrinsics: MacroBody,
        validator_merkle_root: Vec<u8>,
    ) -> MacroBlock {
        let keypair = KeyPair::from(
            SecretKey::deserialize_from_vec(&hex::decode(SECRET_KEY).unwrap()).unwrap(),
        );

        // Create a TendemrintVote instance out of known properties.
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
        let keypair = KeyPair::from(
            SecretKey::deserialize_from_vec(&hex::decode(SECRET_KEY).unwrap()).unwrap(),
        );

        let view_change = ViewChange {
            block_number: self.blockchain.block_number() + 1,
            new_view_number: view_number,
            prev_seed: self.blockchain.head().seed().clone(),
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

// Fill epoch with micro blocks
pub fn fill_micro_blocks(producer: &BlockProducer, blockchain: &Arc<Blockchain>) {
    let init_height = blockchain.block_number();
    let macro_block_number = policy::macro_block_after(init_height + 1);
    for i in (init_height + 1)..macro_block_number {
        let last_micro_block = producer.next_micro_block(
            blockchain.time.now() + i as u64 * 1000,
            0,
            None,
            vec![],
            vec![0x42],
        );
        assert_eq!(
            blockchain.push(Block::Micro(last_micro_block)),
            Ok(PushResult::Extended)
        );
    }
    assert_eq!(blockchain.block_number(), macro_block_number - 1);
}

pub fn sign_macro_block(
    keypair: &KeyPair,
    header: MacroHeader,
    body: Option<MacroBody>,
) -> MacroBlock {
    // Calculate block hash.
    let block_hash = header.hash::<Blake2bHash>();

    // Calculate the validator Merkle root (used in the nano sync).
    let validator_merkle_root =
        pk_tree_construct(vec![keypair.public_key.public_key; policy::SLOTS as usize]);

    // Create the precommit tendermint vote.
    let precommit = TendermintVote {
        proposal_hash: Some(block_hash),
        id: TendermintIdentifier {
            block_number: header.block_number,
            round_number: 0,
            step: TendermintStep::PreCommit,
        },
        validator_merkle_root,
    };

    // Create signed precommit.
    let signed_precommit = keypair.secret_key.sign(&precommit);

    // Create signers Bitset.
    let mut signers = BitSet::new();
    for i in 0..policy::TWO_THIRD_SLOTS {
        signers.insert(i as usize);
    }

    // Create multisignature.
    let multisig = MultiSignature {
        signature: AggregateSignature::from_signatures(&*vec![
            signed_precommit;
            policy::TWO_THIRD_SLOTS as usize
        ]),
        signers,
    };

    // Create Tendermint proof.
    let tendermint_proof = TendermintProof {
        round: 0,
        sig: multisig,
    };

    // Create and return the macro block.
    MacroBlock {
        header,
        body,
        justification: Some(tendermint_proof),
    }
}

// /// Currently unused
// pub fn sign_view_change(
//     keypair: &KeyPair,
//     prev_seed: VrfSeed,
//     block_number: u32,
//     new_view_number: u32,
// ) -> ViewChangeProof {
//     // Create the view change.
//     let view_change = ViewChange {
//         block_number,
//         new_view_number,
//         prev_seed,
//     };

//     // Sign the view change.
//     let signed_view_change =
//         SignedViewChange::from_message(view_change.clone(), &keypair.secret_key, 0).signature;

//     // Create signers Bitset.
//     let mut signers = BitSet::new();
//     for i in 0..policy::TWO_THIRD_SLOTS {
//         signers.insert(i as usize);
//     }

//     // Create ViewChangeProof and return  it.
//     ViewChangeProof::new(
//         AggregateSignature::from_signatures(&*vec![
//             signed_view_change;
//             policy::TWO_THIRD_SLOTS as usize
//         ]),
//         signers,
//     )
// }

pub fn produce_macro_blocks(
    num_macro: usize,
    producer: &BlockProducer,
    blockchain: &Arc<Blockchain>,
) {
    for _ in 0..num_macro {
        fill_micro_blocks(producer, blockchain);

        let _next_block_height = blockchain.block_number() + 1;
        let macro_block = producer.next_macro_block_proposal(
            blockchain.time.now() + blockchain.block_number() as u64 * 1000,
            0u32,
            vec![],
        );

        let block = sign_macro_block(
            &producer.validator_key,
            macro_block.header,
            macro_block.body,
        );
        assert_eq!(
            blockchain.push(Block::Macro(block)),
            Ok(PushResult::Extended)
        );
    }
}
