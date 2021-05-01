use std::sync::Arc;

use beserial::Deserialize;
use nimiq_block_albatross::{
    Block, MacroBlock, MacroBody, MultiSignature, TendermintIdentifier, TendermintProof,
    TendermintProposal, TendermintStep, TendermintVote,
};
use nimiq_block_production_albatross::BlockProducer;
use nimiq_blockchain_albatross::{AbstractBlockchain, Blockchain, PushResult};
use nimiq_bls::{AggregateSignature, KeyPair, SecretKey};
use nimiq_collections::bitset::BitSet;
use nimiq_database::volatile::VolatileEnvironment;
use nimiq_genesis::NetworkId;
use nimiq_hash::{Blake2bHash, Blake2sHash, Hash};
use nimiq_primitives::policy;
use nimiq_primitives::policy::BATCHES_PER_EPOCH;

/// Secret key of validator. Tests run with `genesis/src/genesis/unit-albatross.toml`
const SECRET_KEY: &str = "196ffdb1a8acc7cbd76a251aeac0600a1d68b3aba1eba823b5e4dc5dbdcdc730afa752c05ab4f6ef8518384ad514f403c5a088a22b17bf1bc14f8ff8decc2a512c0a200f68d7bdf5a319b30356fe8d1d75ef510aed7a8660968c216c328a0000";

// Fill epoch with micro blocks
fn fill_micro_blocks(producer: &BlockProducer, blockchain: &Arc<Blockchain>) {
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

fn produce_macro_blocks(num_macro: usize, producer: &BlockProducer, blockchain: &Arc<Blockchain>) {
    for _ in 0..num_macro {
        fill_micro_blocks(producer, blockchain);

        let slots =
            blockchain.get_validators_for_epoch(policy::epoch_at(blockchain.block_number() + 1));
        assert!(slots.is_some());

        let next_block_height = blockchain.block_number() + 1;
        let macro_block_proposal = producer.next_macro_block_proposal(
            blockchain.time.now() + next_block_height as u64 * 1000,
            0u32,
            vec![],
        );

        let block = sign_macro_block(
            TendermintProposal {
                value: macro_block_proposal.header,
                valid_round: None,
            },
            macro_block_proposal
                .body
                .or_else(|| Some(MacroBody::new()))
                .unwrap(),
            MacroBlock::create_pk_tree_root(&slots.unwrap()),
        );
        assert_eq!(
            blockchain.push(Block::Macro(block)),
            Ok(PushResult::Extended)
        );
    }
}

fn sign_macro_block(
    proposal: TendermintProposal,
    extrinsics: MacroBody,
    validator_merkle_root: Vec<u8>,
) -> MacroBlock {
    let keypair =
        KeyPair::from(SecretKey::deserialize_from_vec(&hex::decode(SECRET_KEY).unwrap()).unwrap());

    // Create a TendemrintVote instance out of known properties.
    // round_number is for now fixed at 0 for tests, but it could be anything,
    // as long as the TendermintProof further down this function does use the same round_umber.
    let vote = TendermintVote {
        proposal_hash: Some(proposal.value.hash::<Blake2bHash>()),
        id: TendermintIdentifier {
            block_number: proposal.value.block_number,
            step: TendermintStep::PreCommit,
            round_number: 0,
        },
        validator_merkle_root,
    };

    // create the to be signed hash
    let message_hash = vote.hash::<Blake2sHash>();

    // sign the hash
    let signature = AggregateSignature::from_signatures(&[keypair
        .secret_key
        .sign_hash(message_hash)
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

#[test]
fn it_can_history_sync() {
    // The minimum number of macro blocks necessary so that we have one election block and one
    // checkpoint block to push.
    let num_macro_blocks = (BATCHES_PER_EPOCH + 1) as usize;

    // Create a blockchain to produce the macro blocks.
    let env = VolatileEnvironment::new(10).unwrap();
    let blockchain = Arc::new(Blockchain::new(env, NetworkId::UnitAlbatross).unwrap());

    // Produce the blocks.
    let keypair =
        KeyPair::from(SecretKey::deserialize_from_vec(&hex::decode(SECRET_KEY).unwrap()).unwrap());
    let producer = BlockProducer::new_without_mempool(Arc::clone(&blockchain), keypair);
    produce_macro_blocks(num_macro_blocks, &producer, &blockchain);

    // Get the latest election block and corresponding history tree transactions.
    let election_block = Block::Macro(blockchain.state().election_head.clone());

    let election_txs = blockchain
        .history_store
        .get_epoch_transactions(policy::epoch_at(election_block.block_number()), None);

    // Get the latest checkpoint block and corresponding history tree transactions.
    let checkpoint_block = blockchain.state().macro_info.head.clone();

    let checkpoint_txs = blockchain
        .history_store
        .get_epoch_transactions(policy::epoch_at(checkpoint_block.block_number()), None);

    // Create a second blockchain to push these blocks.
    let env2 = VolatileEnvironment::new(10).unwrap();
    let blockchain2 = Arc::new(Blockchain::new(env2, NetworkId::UnitAlbatross).unwrap());

    // Push blocks using history sync.
    assert_eq!(
        blockchain2.push_history_sync(election_block, &election_txs),
        Ok(PushResult::Extended)
    );

    assert_eq!(
        blockchain2.push_history_sync(checkpoint_block, &checkpoint_txs),
        Ok(PushResult::Extended)
    );
}

// TODO: Test using blocks with transactions.
