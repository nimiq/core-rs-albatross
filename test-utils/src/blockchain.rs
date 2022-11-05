use std::str::FromStr;
use std::sync::Arc;

use parking_lot::RwLock;
use rand::{rngs::StdRng, RngCore, SeedableRng};
use std::time::Instant;

use beserial::Deserialize;
use nimiq_block::{
    Block, MacroBlock, MacroBody, MacroHeader, MultiSignature, SignedSkipBlockInfo, SkipBlockInfo,
    SkipBlockProof, TendermintIdentifier, TendermintProof, TendermintStep, TendermintVote,
};
use nimiq_block_production::BlockProducer;
use nimiq_blockchain::{AbstractBlockchain, Blockchain, PushResult};
use nimiq_bls::{AggregateSignature, KeyPair as BlsKeyPair, SecretKey as BlsSecretKey};
use nimiq_collections::BitSet;
use nimiq_genesis::NetworkId;
use nimiq_keys::{Address, KeyPair as SchnorrKeyPair, PrivateKey as SchnorrPrivateKey};
use nimiq_keys::{KeyPair, PrivateKey};
use nimiq_primitives::coin::Coin;
use nimiq_primitives::policy;
use nimiq_transaction::Transaction;
use nimiq_transaction_builder::TransactionBuilder;

use crate::blockchain_with_rng::*;

/// Secret keys of validator. Tests run with `genesis/src/genesis/unit-albatross.toml`
pub const SIGNING_KEY: &str = "041580cc67e66e9e08b68fd9e4c9deb68737168fbe7488de2638c2e906c2f5ad";
pub const VOTING_KEY: &str = "196ffdb1a8acc7cbd76a251aeac0600a1d68b3aba1eba823b5e4dc5dbdcdc730afa752c05ab4f6ef8518384ad514f403c5a088a22b17bf1bc14f8ff8decc2a512c0a200f68d7bdf5a319b30356fe8d1d75ef510aed7a8660968c216c328a0000";
pub const UNIT_KEY: &str = "6c9320ac201caf1f8eaa5b05f5d67a9e77826f3f6be266a0ecccc20416dc6587";

pub fn generate_transactions(
    key_pair: &KeyPair,
    start_height: u32,
    network_id: NetworkId,
    count: usize,
    rng_seed: u64,
) -> Vec<Transaction> {
    let mut txs = Vec::new();

    let mut rng = StdRng::seed_from_u64(rng_seed);
    for _ in 0..count {
        let mut bytes = [0u8; 20];
        rng.fill_bytes(&mut bytes);
        let recipient = Address::from(bytes);

        let tx = TransactionBuilder::new_basic(
            key_pair,
            recipient,
            Coin::from_u64_unchecked(1),
            Coin::from_u64_unchecked(2),
            start_height,
            network_id,
        )
        .unwrap();
        txs.push(tx);
    }

    txs
}

/// Produces a series of macro blocks (and the corresponding batches).
pub fn produce_macro_blocks(
    producer: &BlockProducer,
    blockchain: &Arc<RwLock<Blockchain>>,
    num_blocks: usize,
) {
    produce_macro_blocks_with_rng(producer, blockchain, num_blocks, &mut rand::thread_rng())
}

/// Produces a series of macro blocks (and the corresponding batches).
pub fn produce_macro_blocks_with_txns(
    producer: &BlockProducer,
    blockchain: &Arc<RwLock<Blockchain>>,
    num_blocks: usize,
    num_txns: usize,
    rng_seed: u64,
) {
    for _ in 0..num_blocks {
        fill_micro_blocks_with_txns(producer, blockchain, num_txns, rng_seed);

        let blockchain = blockchain.upgradable_read();
        let next_block_height = (blockchain.block_number() + 1) as u64;

        let macro_block_proposal = producer.next_macro_block_proposal(
            &blockchain,
            blockchain.time.now() + next_block_height * 1000,
            0u32,
            vec![],
        );

        let block = sign_macro_block(
            &producer.voting_key,
            macro_block_proposal.header,
            macro_block_proposal.body,
        );

        assert_eq!(
            Blockchain::push(blockchain, Block::Macro(block)),
            Ok(PushResult::Extended)
        );
    }
}

/// Create the next micro block with default parameters.
pub fn next_micro_block(producer: &BlockProducer, blockchain: &Arc<RwLock<Blockchain>>) -> Block {
    next_micro_block_with_rng(producer, blockchain, &mut rand::thread_rng())
}

/// Creates and pushes a single micro block to the chain.
pub fn push_micro_block(producer: &BlockProducer, blockchain: &Arc<RwLock<Blockchain>>) -> Block {
    push_micro_block_with_rng(producer, blockchain, &mut rand::thread_rng())
}

/// Fill batch with micro blocks.
pub fn fill_micro_blocks(producer: &BlockProducer, blockchain: &Arc<RwLock<Blockchain>>) {
    fill_micro_blocks_with_rng(producer, blockchain, &mut rand::thread_rng())
}

/// Fill batch with simple transactions to random recipients
pub fn fill_micro_blocks_with_txns(
    producer: &BlockProducer,
    blockchain: &Arc<RwLock<Blockchain>>,
    num_transactions: usize,
    rng_seed: u64,
) {
    let init_height = blockchain.read().block_number();
    let key_pair = KeyPair::from(PrivateKey::from_str(UNIT_KEY).unwrap());
    assert!(policy::is_macro_block_at(init_height));

    let macro_block_number = init_height + policy::BLOCKS_PER_BATCH;

    for i in (init_height + 1)..macro_block_number {
        log::debug!(" Current Height: {}", i);
        let blockchain = blockchain.upgradable_read();

        // Generate the transactions.
        let txns = generate_transactions(
            &key_pair,
            i,
            NetworkId::UnitAlbatross,
            num_transactions,
            rng_seed,
        );
        let start = Instant::now();
        let last_micro_block = producer.next_micro_block(
            &blockchain,
            blockchain.time.now() + i as u64 * 100,
            vec![],
            txns,
            vec![0x42],
            None,
        );
        let duration = start.elapsed();
        log::debug!(
            "   Time elapsed producing micro: {} ms, ",
            duration.as_millis(),
        );

        let start = Instant::now();
        assert_eq!(
            Blockchain::push(blockchain, Block::Micro(last_micro_block)),
            Ok(PushResult::Extended)
        );
        let duration = start.elapsed();
        log::debug!(
            "   Time elapsed pushing micro: {} ms, ",
            duration.as_millis(),
        );
    }

    assert_eq!(blockchain.read().block_number(), macro_block_number - 1);
}

/// Signs a macro block proposal.
pub fn sign_macro_block(
    keypair: &BlsKeyPair,
    header: MacroHeader,
    body: Option<MacroBody>,
) -> MacroBlock {
    // Create the block.
    let mut block = MacroBlock {
        header,
        body,
        justification: None,
    };

    // Calculate block hash.
    let block_hash = block.nano_zkp_hash();

    // Create the precommit tendermint vote.
    let precommit = TendermintVote {
        proposal_hash: Some(block_hash),
        id: TendermintIdentifier {
            block_number: block.block_number(),
            round_number: 0,
            step: TendermintStep::PreCommit,
        },
    };

    // Create signed precommit.
    let signed_precommit = keypair.secret_key.sign(&precommit);

    // Create signers Bitset.
    let mut signers = BitSet::new();
    for i in 0..policy::TWO_F_PLUS_ONE {
        signers.insert(i as usize);
    }

    // Create multisignature.
    let multisig = MultiSignature {
        signature: AggregateSignature::from_signatures(&vec![
            signed_precommit;
            policy::TWO_F_PLUS_ONE as usize
        ]),
        signers,
    };

    // Create Tendermint proof.
    let tendermint_proof = TendermintProof {
        round: 0,
        sig: multisig,
    };

    // Add the justification and return the macro block.
    block.justification = Some(tendermint_proof);

    block
}

pub fn sign_skip_block_info(
    voting_key_pair: &BlsKeyPair,
    skip_block_info: &SkipBlockInfo,
) -> SkipBlockProof {
    let skip_block_info =
        SignedSkipBlockInfo::from_message(skip_block_info.clone(), &voting_key_pair.secret_key, 0);

    let signature =
        AggregateSignature::from_signatures(&[skip_block_info.signature.multiply(policy::SLOTS)]);
    let mut signers = BitSet::new();
    for i in 0..policy::SLOTS {
        signers.insert(i as usize);
    }

    SkipBlockProof {
        sig: MultiSignature::new(signature, signers),
    }
}

pub fn voting_key() -> BlsKeyPair {
    BlsKeyPair::from(BlsSecretKey::deserialize_from_vec(&hex::decode(VOTING_KEY).unwrap()).unwrap())
}

pub fn signing_key() -> SchnorrKeyPair {
    SchnorrKeyPair::from(
        SchnorrPrivateKey::deserialize_from_vec(&hex::decode(SIGNING_KEY).unwrap()).unwrap(),
    )
}
