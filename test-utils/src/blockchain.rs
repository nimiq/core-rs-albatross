use std::{str::FromStr, sync::Arc, time::Instant};

use nimiq_account::{Account, StakingContractStoreWrite, TransactionLog};
use nimiq_block::{
    Block, MacroBlock, MacroBody, MacroHeader, MultiSignature, SignedSkipBlockInfo, SkipBlockInfo,
    SkipBlockProof, TendermintProof,
};
use nimiq_blockchain::{BlockProducer, Blockchain};
use nimiq_blockchain_interface::{AbstractBlockchain, PushResult};
use nimiq_bls::{AggregateSignature, KeyPair as BlsKeyPair, SecretKey as BlsSecretKey};
use nimiq_collections::BitSet;
use nimiq_database::traits::WriteTransaction;
use nimiq_genesis::NetworkId;
use nimiq_hash::Blake2bHash;
use nimiq_keys::{
    Address, KeyPair as SchnorrKeyPair, KeyPair, PrivateKey as SchnorrPrivateKey, PrivateKey,
};
use nimiq_primitives::{
    coin::Coin, key_nibbles::KeyNibbles, policy::Policy, TendermintIdentifier, TendermintStep,
    TendermintVote,
};
use nimiq_serde::Deserialize;
use nimiq_transaction::Transaction;
use nimiq_transaction_builder::TransactionBuilder;
use parking_lot::RwLock;
use rand::{rngs::StdRng, Rng, SeedableRng};

use crate::{
    blockchain_with_rng::*,
    test_custom_block::{next_macro_block, BlockConfig},
    test_rng,
};

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

        let macro_block_proposal = producer
            .next_macro_block_proposal(
                &blockchain,
                blockchain.timestamp() + Policy::BLOCK_SEPARATION_TIME,
                0u32,
                vec![],
                None,
            )
            .unwrap();

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

    let macro_block_number = Policy::macro_block_after(init_height + 1);

    for i in (init_height + 1)..macro_block_number {
        let blockchain = blockchain.upgradable_read();

        // Generate the transactions.
        let txns = generate_transactions(
            &key_pair,
            i,
            NetworkId::UnitAlbatross,
            num_transactions,
            rng_seed,
        );

        let block_production_start = Instant::now();
        let last_micro_block = producer
            .next_micro_block(
                &blockchain,
                blockchain.timestamp() + Policy::BLOCK_SEPARATION_TIME,
                vec![],
                txns,
                vec![0x42],
                None,
            )
            .unwrap();
        let block_production_duration = block_production_start.elapsed();

        let block_push_start = Instant::now();
        assert_eq!(
            Blockchain::push(blockchain, Block::Micro(last_micro_block)),
            Ok(PushResult::Extended)
        );
        let block_push_duration = block_push_start.elapsed();
        log::info!(
            " Pushed block {}, Time producing block {}ms, Time pushing block {}ms",
            i,
            block_production_duration.as_millis(),
            block_push_duration.as_millis(),
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
            network: block.network(),
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

pub fn signal_all_validators_support_for_next_version(blockchain: &Arc<RwLock<Blockchain>>) -> u16 {
    let blockchain_read = blockchain.read();
    let next_version = blockchain_read.state.current_version() + 1;
    assert!(next_version <= Policy::max_supported_version());

    let current_validators = blockchain_read
        .current_validators()
        .expect("Current validator set must exist")
        .clone();
    let mut staking_contract = blockchain_read.get_staking_contract();

    let mut raw_txn = blockchain_read.write_transaction();
    let data_store = blockchain_read
        .state()
        .accounts
        .data_store(&Policy::STAKING_CONTRACT_ADDRESS);
    let mut txn = (&mut raw_txn).into();
    let mut data_store_write = data_store.write(&mut txn);
    let mut store = StakingContractStoreWrite::new(&mut data_store_write);

    let version_bytes = next_version.to_be_bytes();
    let mut signal_data = [0; 32];
    signal_data[..2].copy_from_slice(&version_bytes);
    let signal_data = Blake2bHash(signal_data);

    for validator in current_validators.iter() {
        staking_contract
            .update_validator(
                &mut store,
                &validator.address,
                None,
                None,
                None,
                Some(Some(signal_data.clone())),
                &mut TransactionLog::empty(),
            )
            .expect("Failed to update validator");
    }

    blockchain_read
        .state()
        .accounts
        .tree
        .put(
            &mut txn,
            &KeyNibbles::from(&Policy::STAKING_CONTRACT_ADDRESS),
            Account::Staking(staking_contract),
        )
        .expect("Failed to persist staking contract");

    raw_txn.commit();

    next_version
}

pub fn next_election_block_with_version(
    producer: &BlockProducer,
    blockchain: &Arc<RwLock<Blockchain>>,
    version: u16,
) -> Block {
    let blockchain = blockchain.read();
    next_macro_block(
        &producer.signing_key,
        &producer.voting_key,
        &blockchain,
        &BlockConfig {
            version: Some(version),
            ..Default::default()
        },
    )
}

pub fn next_protocol_upgrade_block(
    producer: &BlockProducer,
    blockchain: &Arc<RwLock<Blockchain>>,
) -> (Block, u16) {
    let next_version = signal_all_validators_support_for_next_version(blockchain);
    (
        next_election_block_with_version(producer, blockchain, next_version),
        next_version,
    )
}

pub fn signal_next_protocol_version_via_tx(
    producer: &BlockProducer,
    blockchain: &Arc<RwLock<Blockchain>>,
) -> u16 {
    let next_version = blockchain.read().state.current_version() + 1;
    assert!(next_version <= Policy::max_supported_version());

    let version_bytes = next_version.to_be_bytes();
    let mut signal_data = [0; 32];
    signal_data[..2].copy_from_slice(&version_bytes);

    let tx = TransactionBuilder::new_update_validator(
        &validator_key(),
        &validator_key(),
        None,
        None,
        None,
        Some(Some(Blake2bHash(signal_data))),
        Coin::ZERO,
        blockchain.read().block_number(),
        NetworkId::UnitAlbatross,
    );

    let block = producer
        .next_micro_block(
            &blockchain.read(),
            blockchain.read().timestamp() + Policy::BLOCK_SEPARATION_TIME,
            vec![],
            vec![tx],
            vec![0x42],
            None,
        )
        .expect("Failed to create signal transaction block");

    assert_eq!(
        Blockchain::push(blockchain.upgradable_read(), Block::Micro(block)),
        Ok(PushResult::Extended)
    );

    next_version
}

pub fn voting_key() -> BlsKeyPair {
    BlsKeyPair::from(BlsSecretKey::deserialize_from_vec(&hex::decode(VOTING_KEY).unwrap()).unwrap())
}

pub fn signing_key() -> SchnorrKeyPair {
    SchnorrKeyPair::from(
        SchnorrPrivateKey::deserialize_from_vec(&hex::decode(SIGNING_KEY).unwrap()).unwrap(),
    )
}
