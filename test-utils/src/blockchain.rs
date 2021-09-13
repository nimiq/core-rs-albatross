use std::sync::Arc;

use parking_lot::RwLock;

use beserial::Deserialize;
use nimiq_block::{
    Block, MacroBlock, MacroBody, MacroHeader, MultiSignature, SignedViewChange,
    TendermintIdentifier, TendermintProof, TendermintStep, TendermintVote, ViewChange,
    ViewChangeProof,
};
use nimiq_block_production::BlockProducer;
use nimiq_blockchain::{AbstractBlockchain, Blockchain, PushResult};
use nimiq_bls::{AggregateSignature, KeyPair, SecretKey};
use nimiq_collections::BitSet;
use nimiq_hash::{Blake2bHash, Hash};
use nimiq_nano_primitives::pk_tree_construct;
use nimiq_primitives::policy;
use nimiq_vrf::VrfSeed;

/// Secret key of validator. Tests run with `genesis/src/genesis/unit-albatross.toml`
pub const SECRET_KEY: &str = "196ffdb1a8acc7cbd76a251aeac0600a1d68b3aba1eba823b5e4dc5dbdcdc730afa752c05ab4f6ef8518384ad514f403c5a088a22b17bf1bc14f8ff8decc2a512c0a200f68d7bdf5a319b30356fe8d1d75ef510aed7a8660968c216c328a0000";

// Produces a series of macro blocks (and the corresponding batches).
pub fn produce_macro_blocks(
    num_macro: usize,
    producer: &BlockProducer,
    blockchain: &Arc<RwLock<Blockchain>>,
) {
    for _ in 0..num_macro {
        fill_micro_blocks(producer, blockchain);

        let blockchain = blockchain.upgradable_read();
        let next_block_height = (blockchain.block_number() + 1) as u64;

        let macro_block_proposal = producer.next_macro_block_proposal(
            blockchain.time.now() + next_block_height * 1000,
            0u32,
            vec![],
        );

        let block = sign_macro_block(
            &producer.validator_key,
            macro_block_proposal.header,
            macro_block_proposal.body,
        );

        assert_eq!(
            Blockchain::push(blockchain, Block::Macro(block)),
            Ok(PushResult::Extended)
        );
    }
}

// Fill batch with micro blocks.
pub fn fill_micro_blocks(producer: &BlockProducer, blockchain: &Arc<RwLock<Blockchain>>) {
    let init_height = blockchain.read().block_number();

    assert!(policy::is_macro_block_at(init_height));

    let macro_block_number = init_height + policy::BATCH_LENGTH;

    for i in (init_height + 1)..macro_block_number {
        let blockchain = blockchain.upgradable_read();
        let last_micro_block = producer.next_micro_block(
            blockchain.time.now() + i as u64 * 1000,
            0,
            None,
            vec![],
            vec![0x42],
        );

        assert_eq!(
            Blockchain::push(blockchain, Block::Micro(last_micro_block)),
            Ok(PushResult::Extended)
        );
    }

    assert_eq!(blockchain.read().block_number(), macro_block_number - 1);
}

// Signs a macro block proposal.
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

pub fn sign_view_change(
    prev_seed: VrfSeed,
    block_number: u32,
    new_view_number: u32,
) -> ViewChangeProof {
    let keypair =
        KeyPair::from(SecretKey::deserialize_from_vec(&hex::decode(SECRET_KEY).unwrap()).unwrap());

    // Create the view change.
    let view_change = ViewChange {
        block_number,
        new_view_number,
        prev_seed,
    };

    // Sign the view change.
    let signed_view_change =
        SignedViewChange::from_message(view_change, &keypair.secret_key, 0).signature;

    // Create signers Bitset.
    let mut signers = BitSet::new();
    for i in 0..policy::TWO_THIRD_SLOTS {
        signers.insert(i as usize);
    }

    // Create multisignature.
    let multisig = MultiSignature {
        signature: AggregateSignature::from_signatures(&*vec![
            signed_view_change;
            policy::TWO_THIRD_SLOTS as usize
        ]),
        signers,
    };

    // Create and return view change proof.
    ViewChangeProof { sig: multisig }
}
