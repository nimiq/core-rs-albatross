use std::sync::Arc;

use beserial::Deserialize;
use nimiq_block::{
    Block, MacroBlock, MacroBody, MultiSignature, TendermintIdentifier, TendermintProof,
    TendermintProposal, TendermintStep, TendermintVote,
};
use nimiq_block_production::BlockProducer;
use nimiq_blockchain::{AbstractBlockchain, Blockchain, PushResult};
use nimiq_bls::{AggregateSignature, KeyPair, SecretKey};
use nimiq_collections::bitset::BitSet;
use nimiq_database::volatile::VolatileEnvironment;
use nimiq_genesis::NetworkId;
use nimiq_hash::{Blake2bHash, Blake2sHash, Hash};
use nimiq_primitives::policy;
use nimiq_primitives::policy::{BATCHES_PER_EPOCH, BATCH_LENGTH, EPOCH_LENGTH};

// Secret key of validator. Tests run with `genesis/src/genesis/unit-albatross.toml`
const SECRET_KEY: &str = "196ffdb1a8acc7cbd76a251aeac0600a1d68b3aba1eba823b5e4dc5dbdcdc730afa752c05ab4f6ef8518384ad514f403c5a088a22b17bf1bc14f8ff8decc2a512c0a200f68d7bdf5a319b30356fe8d1d75ef510aed7a8660968c216c328a0000";

// Produces a series of macro blocks (and the corresponding batches).
fn produce_macro_blocks(num_macro: usize, producer: &BlockProducer, blockchain: &Arc<Blockchain>) {
    for _ in 0..num_macro {
        fill_micro_blocks(producer, blockchain);

        let slots = blockchain.current_validators().unwrap();

        let next_block_height = (blockchain.block_number() + 1) as u64;

        let macro_block_proposal = producer.next_macro_block_proposal(
            blockchain.time.now() + next_block_height * 1000,
            0u32,
            vec![],
        );

        let block = sign_macro_block(
            TendermintProposal {
                value: macro_block_proposal.header,
                valid_round: None,
            },
            macro_block_proposal.body.unwrap(),
            MacroBlock::create_pk_tree_root(&slots),
        );

        assert_eq!(
            blockchain.push(Block::Macro(block)),
            Ok(PushResult::Extended)
        );
    }
}

// Fill batch with micro blocks.
fn fill_micro_blocks(producer: &BlockProducer, blockchain: &Arc<Blockchain>) {
    let init_height = blockchain.block_number();

    assert!(policy::is_macro_block_at(init_height));

    let macro_block_number = init_height + BATCH_LENGTH;

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

// Signs a macro block proposal.
fn sign_macro_block(
    proposal: TendermintProposal,
    extrinsics: MacroBody,
    validator_merkle_root: Vec<u8>,
) -> MacroBlock {
    let keypair =
        KeyPair::from(SecretKey::deserialize_from_vec(&hex::decode(SECRET_KEY).unwrap()).unwrap());

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

    // create the to-be-signed hash
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

// Tests if the basic history sync works. It will try to push a succession of election and checkpoint
// blocks. It does test if election blocks can be pushed after checkpoint blocks and vice-versa. It
// does NOT test if macro blocks can be pushed with micro blocks already in the blockchain.
#[test]
fn history_sync_works() {
    // The minimum number of macro blocks necessary so that we have two election blocks and a few
    // checkpoint blocks to push.
    let num_macro_blocks = (2 * BATCHES_PER_EPOCH + 1) as usize;

    // Create a blockchain to produce the macro blocks.
    let env = VolatileEnvironment::new(10).unwrap();

    let blockchain = Arc::new(Blockchain::new(env, NetworkId::UnitAlbatross).unwrap());

    // Produce the blocks.
    let keypair =
        KeyPair::from(SecretKey::deserialize_from_vec(&hex::decode(SECRET_KEY).unwrap()).unwrap());

    let producer = BlockProducer::new_without_mempool(Arc::clone(&blockchain), keypair);

    produce_macro_blocks(num_macro_blocks, &producer, &blockchain);

    // Get the election blocks and corresponding history tree transactions.
    let election_block_1 = blockchain
        .chain_store
        .get_block_at(EPOCH_LENGTH, true, None)
        .unwrap();

    let election_txs_1 = blockchain.history_store.get_epoch_transactions(1, None);

    let election_block_2 = blockchain
        .chain_store
        .get_block_at(2 * EPOCH_LENGTH, true, None)
        .unwrap();

    let election_txs_2 = blockchain.history_store.get_epoch_transactions(2, None);

    // Get the checkpoint blocks and corresponding history tree transactions.
    let checkpoint_block_2_1 = blockchain
        .chain_store
        .get_block_at(EPOCH_LENGTH + BATCH_LENGTH, true, None)
        .unwrap();

    let mut checkpoint_txs_2_1 = vec![];

    for ext_tx in &election_txs_2 {
        if ext_tx.block_number > EPOCH_LENGTH + BATCH_LENGTH {
            break;
        }

        checkpoint_txs_2_1.push(ext_tx.clone());
    }

    let checkpoint_block_2_3 = blockchain
        .chain_store
        .get_block_at(EPOCH_LENGTH + 3 * BATCH_LENGTH, true, None)
        .unwrap();

    let mut checkpoint_txs_2_3 = vec![];

    for ext_tx in &election_txs_2 {
        if ext_tx.block_number > EPOCH_LENGTH + 3 * BATCH_LENGTH {
            break;
        }

        checkpoint_txs_2_3.push(ext_tx.clone());
    }

    let checkpoint_block_3_1 = blockchain
        .chain_store
        .get_block_at(2 * EPOCH_LENGTH + BATCH_LENGTH, true, None)
        .unwrap();

    let checkpoint_txs_3_1 = blockchain.history_store.get_epoch_transactions(3, None);

    // Create a second blockchain to push these blocks.
    let env2 = VolatileEnvironment::new(10).unwrap();

    let blockchain2 = Arc::new(Blockchain::new(env2, NetworkId::UnitAlbatross).unwrap());

    // Push blocks using history sync.
    assert_eq!(
        blockchain2.push_history_sync(election_block_1, &election_txs_1),
        Ok(PushResult::Extended)
    );

    assert_eq!(
        blockchain2.push_history_sync(checkpoint_block_2_1, &checkpoint_txs_2_1),
        Ok(PushResult::Extended)
    );

    assert_eq!(
        blockchain2.push_history_sync(checkpoint_block_2_3, &checkpoint_txs_2_3),
        Ok(PushResult::Extended)
    );

    assert_eq!(
        blockchain2.push_history_sync(election_block_2, &election_txs_2),
        Ok(PushResult::Extended)
    );

    assert_eq!(
        blockchain2.push_history_sync(checkpoint_block_3_1, &checkpoint_txs_3_1),
        Ok(PushResult::Extended)
    );
}

// Tests if the history sync works when micro blocks have already been pushed in the blockchain.
// This basically tests if we can go from the history sync to the normal follow mode and back.
#[test]
fn history_sync_works_with_micro_blocks() {
    // The minimum number of macro blocks necessary so that we have two election blocks and a few
    // checkpoint blocks to push.
    let num_macro_blocks = (2 * BATCHES_PER_EPOCH + 2) as usize;

    // Create a blockchain to produce the macro blocks.
    let env = VolatileEnvironment::new(10).unwrap();

    let blockchain = Arc::new(Blockchain::new(env, NetworkId::UnitAlbatross).unwrap());

    // Produce the blocks.
    let keypair =
        KeyPair::from(SecretKey::deserialize_from_vec(&hex::decode(SECRET_KEY).unwrap()).unwrap());

    let producer = BlockProducer::new_without_mempool(Arc::clone(&blockchain), keypair);

    produce_macro_blocks(num_macro_blocks, &producer, &blockchain);

    // Get the election blocks and corresponding history tree transactions.
    let election_block_1 = blockchain
        .chain_store
        .get_block_at(EPOCH_LENGTH, true, None)
        .unwrap();

    let election_txs_1 = blockchain.history_store.get_epoch_transactions(1, None);

    let election_block_2 = blockchain
        .chain_store
        .get_block_at(2 * EPOCH_LENGTH, true, None)
        .unwrap();

    let election_txs_2 = blockchain.history_store.get_epoch_transactions(2, None);

    // Get the checkpoint blocks and corresponding history tree transactions.
    let checkpoint_block_2_1 = blockchain
        .chain_store
        .get_block_at(EPOCH_LENGTH + BATCH_LENGTH, true, None)
        .unwrap();

    let mut checkpoint_txs_2_1 = vec![];

    for ext_tx in &election_txs_2 {
        if ext_tx.block_number > EPOCH_LENGTH + BATCH_LENGTH {
            break;
        }

        checkpoint_txs_2_1.push(ext_tx.clone());
    }

    let checkpoint_block_3_2 = blockchain
        .chain_store
        .get_block_at(2 * EPOCH_LENGTH + 2 * BATCH_LENGTH, true, None)
        .unwrap();

    let checkpoint_txs_3_2 = blockchain.history_store.get_epoch_transactions(3, None);

    // Get the micro blocks.
    let mut micro_blocks_2_2 = vec![];

    for i in 1..BATCH_LENGTH {
        micro_blocks_2_2.push(
            blockchain
                .chain_store
                .get_block_at(EPOCH_LENGTH + BATCH_LENGTH + i, true, None)
                .unwrap(),
        )
    }

    let mut micro_blocks_3_1 = vec![];

    for i in 1..BATCH_LENGTH {
        micro_blocks_3_1.push(
            blockchain
                .chain_store
                .get_block_at(2 * EPOCH_LENGTH + i, true, None)
                .unwrap(),
        )
    }

    // Create a second blockchain to push these blocks.
    let env2 = VolatileEnvironment::new(10).unwrap();

    let blockchain2 = Arc::new(Blockchain::new(env2, NetworkId::UnitAlbatross).unwrap());

    // Push blocks using history sync.
    assert_eq!(
        blockchain2.push_history_sync(election_block_1, &election_txs_1),
        Ok(PushResult::Extended)
    );

    assert_eq!(
        blockchain2.push_history_sync(checkpoint_block_2_1, &checkpoint_txs_2_1),
        Ok(PushResult::Extended)
    );

    // Now go into follow mode.
    for micro in micro_blocks_2_2 {
        assert_eq!(blockchain2.push(micro,), Ok(PushResult::Extended));
    }

    // Now go back into history sync.
    assert_eq!(
        blockchain2.push_history_sync(election_block_2, &election_txs_2),
        Ok(PushResult::Extended)
    );

    // Into follow mode one more time.
    for micro in micro_blocks_3_1 {
        assert_eq!(blockchain2.push(micro,), Ok(PushResult::Extended));
    }

    // End by going into history sync.
    assert_eq!(
        blockchain2.push_history_sync(checkpoint_block_3_2, &checkpoint_txs_3_2),
        Ok(PushResult::Extended)
    );
}
