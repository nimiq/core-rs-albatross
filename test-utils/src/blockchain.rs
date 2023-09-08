use std::{str::FromStr, sync::Arc, time::Instant};

use nimiq_block::{
    Block, MacroBlock, MacroBody, MacroHeader, MultiSignature, SignedSkipBlockInfo, SkipBlockInfo,
    SkipBlockProof, TendermintIdentifier, TendermintProof, TendermintStep, TendermintVote,
};
use nimiq_block_production::BlockProducer;
use nimiq_blockchain::Blockchain;
use nimiq_blockchain_interface::{AbstractBlockchain, PushResult};
use nimiq_bls::{AggregateSignature, KeyPair as BlsKeyPair, SecretKey as BlsSecretKey};
use nimiq_collections::BitSet;
use nimiq_genesis::NetworkId;
use nimiq_keys::{
    Address, KeyPair as SchnorrKeyPair, KeyPair, PrivateKey as SchnorrPrivateKey, PrivateKey,
};
use nimiq_primitives::{coin::Coin, policy::Policy};
use nimiq_serde::Deserialize;
use nimiq_transaction::Transaction;
use nimiq_transaction_builder::TransactionBuilder;
use parking_lot::RwLock;
use rand::{rngs::StdRng, RngCore, SeedableRng};

use crate::{blockchain_with_rng::*, test_rng::test_rng};

/// Secret keys of validator. Tests run with `genesis/src/genesis/unit-albatross.toml`
pub const SIGNING_KEY: &str = "041580cc67e66e9e08b68fd9e4c9deb68737168fbe7488de2638c2e906c2f5ad";
pub const VOTING_KEY: &str = "99237809f3b37bd0878854d2b5b66e4cc00ba1a1d64377c374f2b6d1bf3dec7835bfae3e7ab89b6d331b3ef7d1e9a06a7f6967bf00edf9e0bcfe34b58bd1260e96406e09156e4c190ff8f69a9ce1183b4289383e6d798fd5104a3800fabd00";
pub const REWARD_KEY: &str = "6c9320ac201caf1f8eaa5b05f5d67a9e77826f3f6be266a0ecccc20416dc6587";
pub const VALIDATOR_KEY: &str = "6927eb8de74e8ea06a8afae5a66db176a7031f742b656651ac53bddb8a4ad3f3";

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
    produce_macro_blocks_with_rng(producer, blockchain, num_blocks, &mut test_rng(false))
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

        let macro_block_proposal = producer.next_macro_block_proposal(
            &blockchain,
            blockchain.head().timestamp() + Policy::BLOCK_SEPARATION_TIME,
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
    next_micro_block_with_rng(producer, blockchain, &mut test_rng(false))
}

/// Creates and pushes a single micro block to the chain.
pub fn push_micro_block(producer: &BlockProducer, blockchain: &Arc<RwLock<Blockchain>>) -> Block {
    push_micro_block_with_rng(producer, blockchain, &mut test_rng(false))
}

/// Fill batch with micro blocks.
pub fn fill_micro_blocks(producer: &BlockProducer, blockchain: &Arc<RwLock<Blockchain>>) {
    fill_micro_blocks_with_rng(producer, blockchain, &mut test_rng(false))
}

/// Fill batch with simple transactions to random recipients
pub fn fill_micro_blocks_with_txns(
    producer: &BlockProducer,
    blockchain: &Arc<RwLock<Blockchain>>,
    num_transactions: usize,
    rng_seed: u64,
) {
    let init_height = blockchain.read().block_number();
    let key_pair = KeyPair::from(PrivateKey::from_str(REWARD_KEY).unwrap());
    assert!(Policy::is_macro_block_at(init_height));

    let macro_block_number = init_height + Policy::blocks_per_batch();

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
            blockchain.head().timestamp() + Policy::BLOCK_SEPARATION_TIME,
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
    let block_hash = block.hash_blake2s();

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
    for i in 0..Policy::TWO_F_PLUS_ONE {
        signers.insert(i as usize);
    }

    // Create multisignature.
    let multisig = MultiSignature {
        signature: AggregateSignature::from_signatures(&vec![
            signed_precommit;
            Policy::TWO_F_PLUS_ONE as usize
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
        AggregateSignature::from_signatures(&[skip_block_info.signature.multiply(Policy::SLOTS)]);
    let mut signers = BitSet::new();
    for i in 0..Policy::SLOTS {
        signers.insert(i as usize);
    }

    SkipBlockProof {
        sig: MultiSignature::new(signature, signers),
    }
}

pub fn validator_key() -> SchnorrKeyPair {
    SchnorrKeyPair::from(
        SchnorrPrivateKey::deserialize_from_vec(&hex::decode(VALIDATOR_KEY).unwrap()).unwrap(),
    )
}

pub fn validator_address() -> Address {
    Address::from(&validator_key())
}

pub fn voting_key() -> BlsKeyPair {
    BlsKeyPair::from(BlsSecretKey::deserialize_from_vec(&hex::decode(VOTING_KEY).unwrap()).unwrap())
}

pub fn signing_key() -> SchnorrKeyPair {
    SchnorrKeyPair::from(
        SchnorrPrivateKey::deserialize_from_vec(&hex::decode(SIGNING_KEY).unwrap()).unwrap(),
    )
}
