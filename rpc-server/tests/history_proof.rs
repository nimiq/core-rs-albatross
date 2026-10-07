use std::{str::FromStr, sync::Arc, time::Duration};

use nimiq_block::Block;
use nimiq_blockchain::{interface::HistoryInterface, BlockProducer, Blockchain, BlockchainConfig};
use nimiq_blockchain_interface::{AbstractBlockchain, PushResult};
use nimiq_database::mdbx::MdbxDatabase;
use nimiq_hash::{Blake2bHash, Hash, HashOutput};
use nimiq_keys::{Address, KeyPair, PrivateKey};
use nimiq_mmr::mmr::proof::Proof;
use nimiq_primitives::{
    account::AccountType,
    coin::Coin,
    networks::NetworkId,
    policy::{upgrades, Policy},
};
use nimiq_rpc_interface::{blockchain::BlockchainInterface, types::HistoryProofData};
use nimiq_rpc_server::{dispatchers::BlockchainDispatcher, Error};
use nimiq_serde::{Deserialize, Serialize};
use nimiq_test_log::test;
use nimiq_test_utils::{
    block_production::TemporaryBlockProducer,
    blockchain::{
        produce_macro_blocks_with_txns, sign_macro_block, signing_key, voting_key, REWARD_KEY,
    },
    test_rng,
};
use nimiq_transaction::{
    account::htlc_contract::{AnyHash, AnyHash32},
    bridge_contract::{AddressFormat, ChainConfig, Endianness, ValidationOp, ValidationProgram},
    historic_transaction::{HistoricTransaction, HistoricTransactionData},
    SignatureProof, Transaction,
};
use nimiq_transaction_builder::TransactionBuilder;
use nimiq_utils::time::OffsetTime;
use parking_lot::RwLock;
use rand::rngs::StdRng;
use serde_json::json;

const HISTORY_ROOT_OFFSET: usize = 307;
const PARENT_ELECTION_HASH_OFFSET: usize = 51;

fn blockchain(index_history: bool) -> Arc<RwLock<Blockchain>> {
    Arc::new(RwLock::new(
        Blockchain::new(
            MdbxDatabase::new_volatile(Default::default()).unwrap(),
            BlockchainConfig {
                index_history,
                ..Default::default()
            },
            NetworkId::UnitAlbatross,
            Arc::new(OffsetTime::new()),
        )
        .unwrap(),
    ))
}

/// A chain with transactions in every micro block, through the first election block and the
/// first checkpoint block after it.
fn chain_with_transactions() -> Arc<RwLock<Blockchain>> {
    let blockchain = blockchain(true);
    let producer = BlockProducer::new(signing_key(), voting_key());
    produce_macro_blocks_with_txns(
        &producer,
        &blockchain,
        Policy::batches_per_epoch() as usize + 1,
        2,
        7,
    );
    blockchain
}

fn macro_block(blockchain: &Arc<RwLock<Blockchain>>, block_number: u32) -> Block {
    let block = blockchain
        .read()
        .chain_store
        .get_block_at(block_number, false, None)
        .unwrap();
    assert!(block.is_macro());
    block
}

/// The hash of a basic transaction in the given block.
fn transaction_in(blockchain: &Arc<RwLock<Blockchain>>, block_number: u32) -> Blake2bHash {
    blockchain
        .read()
        .history_store
        .get_block_transactions(block_number, None)
        .into_iter()
        .find(|tx| matches!(tx.data, HistoricTransactionData::Basic(_)))
        .expect("block has a transaction")
        .tx_hash()
        .into()
}

/// Checks a served proof the way an independent verifier would: from the served bytes only.
fn assert_proof_verifies(proof: &HistoryProofData, tx_hash: &Blake2bHash, block: &Block) {
    assert_eq!(proof.block_number, block.block_number());
    assert_eq!(&proof.history_root, block.history_root());

    let hist_tx = HistoricTransaction::deserialize_from_vec(
        &hex::decode(&proof.historic_transaction).unwrap(),
    )
    .unwrap();
    assert_eq!(&Blake2bHash::from(hist_tx.tx_hash()), tx_hash);

    let mmr_proof = Proof {
        mmr_size: proof.mmr_size as usize,
        nodes: proof.nodes.clone(),
    };
    assert_eq!(
        mmr_proof.verify(
            &proof.history_root,
            &[(proof.leaf_index as usize, &hist_tx)]
        ),
        Ok(true)
    );
}

#[test(tokio::test)]
async fn history_proof_verifies_against_every_later_macro_block_of_the_epoch() {
    let blockchain = chain_with_transactions();
    let dispatcher = BlockchainDispatcher::new(blockchain.clone().into());

    let first_block = Policy::genesis_block_number() + 1;
    let tx_hash = transaction_in(&blockchain, first_block);
    let election = Policy::election_block_after(first_block);

    let mut previous_size = 0;
    let mut macro_number = Policy::macro_block_after(first_block);
    while macro_number <= election {
        let block = macro_block(&blockchain, macro_number);
        let proof = dispatcher
            .get_transaction_history_proof(tx_hash.clone(), macro_number)
            .await
            .unwrap()
            .data;
        assert_proof_verifies(&proof, &tx_hash, &block);

        // A checkpoint proof is against the tree as it was at that block, so the tree grows.
        assert!(proof.mmr_size > previous_size);
        previous_size = proof.mmr_size;
        macro_number += Policy::blocks_per_batch();
    }
}

#[test(tokio::test)]
async fn history_proof_covers_a_transaction_in_the_last_batch_before_the_macro_block() {
    let blockchain = chain_with_transactions();
    let dispatcher = BlockchainDispatcher::new(blockchain.clone().into());

    let checkpoint = Policy::genesis_block_number() + 2 * Policy::blocks_per_batch();
    let tx_hash = transaction_in(&blockchain, checkpoint - 1);
    let proof = dispatcher
        .get_transaction_history_proof(tx_hash.clone(), checkpoint)
        .await
        .unwrap()
        .data;
    assert_proof_verifies(&proof, &tx_hash, &macro_block(&blockchain, checkpoint));
}

#[test(tokio::test)]
async fn history_proof_rejects_a_macro_block_before_the_transaction() {
    let blockchain = chain_with_transactions();
    let dispatcher = BlockchainDispatcher::new(blockchain.clone().into());

    let first_checkpoint = Policy::genesis_block_number() + Policy::blocks_per_batch();
    let tx_hash = transaction_in(&blockchain, first_checkpoint + 1);
    assert!(matches!(
        dispatcher
            .get_transaction_history_proof(tx_hash.clone(), first_checkpoint)
            .await,
        Err(Error::TransactionNotInHistoryAt(hash, number))
            if hash == tx_hash && number == first_checkpoint
    ));
}

#[test(tokio::test)]
async fn history_proof_rejects_a_macro_block_of_another_epoch() {
    let blockchain = chain_with_transactions();
    let dispatcher = BlockchainDispatcher::new(blockchain.clone().into());

    let first_block = Policy::genesis_block_number() + 1;
    let tx_hash = transaction_in(&blockchain, first_block);
    let next_epoch_checkpoint =
        Policy::election_block_after(first_block) + Policy::blocks_per_batch();
    assert!(matches!(
        dispatcher
            .get_transaction_history_proof(tx_hash, next_epoch_checkpoint)
            .await,
        Err(Error::TransactionNotInHistoryAt(..))
    ));
}

#[test(tokio::test)]
async fn history_proof_rejects_bad_requests() {
    let blockchain = chain_with_transactions();
    let dispatcher = BlockchainDispatcher::new(blockchain.clone().into());

    let first_block = Policy::genesis_block_number() + 1;
    let tx_hash = transaction_in(&blockchain, first_block);
    let checkpoint = Policy::macro_block_after(first_block);

    assert!(matches!(
        dispatcher
            .get_transaction_history_proof(tx_hash.clone(), checkpoint + 1)
            .await,
        Err(Error::NotAMacroBlock(number)) if number == checkpoint + 1
    ));
    assert!(matches!(
        dispatcher
            .get_transaction_history_proof(Blake2bHash::default(), checkpoint)
            .await,
        Err(Error::TransactionNotFound(_))
    ));

    // A macro block number past the head.
    let future = Policy::election_block_after(first_block) + 2 * Policy::blocks_per_batch();
    assert!(matches!(
        dispatcher.get_transaction_history_proof(tx_hash, future).await,
        Err(Error::BlockNotFound(number)) if number == future
    ));
}

#[test(tokio::test)]
async fn history_proof_requires_the_history_index() {
    let dispatcher = BlockchainDispatcher::new(blockchain(false).into());
    assert!(matches!(
        dispatcher
            .get_transaction_history_proof(Blake2bHash::default(), Policy::genesis_block_number())
            .await,
        Err(Error::RequiresHistoryIndex)
    ));
}

#[test(tokio::test)]
async fn raw_macro_header_hashes_to_the_block_hash() {
    let blockchain = chain_with_transactions();
    let dispatcher = BlockchainDispatcher::new(blockchain.clone().into());

    let genesis = Policy::genesis_block_number();
    let last = Policy::election_block_after(genesis + 1) + Policy::blocks_per_batch();
    let mut macro_number = genesis;
    while macro_number <= last {
        let Block::Macro(block) = macro_block(&blockchain, macro_number) else {
            unreachable!()
        };
        let content = hex::decode(
            dispatcher
                .get_raw_macro_header(macro_number)
                .await
                .unwrap()
                .data,
        )
        .unwrap();

        assert_eq!(content.hash::<Blake2bHash>(), block.hash());
        assert_eq!(
            &content[HISTORY_ROOT_OFFSET..HISTORY_ROOT_OFFSET + 32],
            block.header.history_root.as_bytes()
        );
        assert_eq!(
            &content[PARENT_ELECTION_HASH_OFFSET..PARENT_ELECTION_HASH_OFFSET + 32],
            block.header.parent_election_hash.as_bytes()
        );
        // Version 2 added `diff_root` at the end.
        let expected_len = if block.header.version >= 2 { 371 } else { 339 };
        assert_eq!(content.len(), expected_len);
        macro_number += Policy::blocks_per_batch();
    }
}

#[test(tokio::test)]
async fn raw_macro_header_rejects_micro_and_missing_blocks() {
    let blockchain = chain_with_transactions();
    let dispatcher = BlockchainDispatcher::new(blockchain.into());

    let micro = Policy::genesis_block_number() + 1;
    assert!(matches!(
        dispatcher.get_raw_macro_header(micro).await,
        Err(Error::NotAMacroBlock(number)) if number == micro
    ));

    let future = Policy::election_block_after(micro) + 2 * Policy::blocks_per_batch();
    assert!(matches!(
        dispatcher.get_raw_macro_header(future).await,
        Err(Error::BlockNotFound(number)) if number == future
    ));
}

const EVM_CHAIN_ID: u64 = 31337;
const EVM_BRIDGE: &str = "5fbdb2315678afecb367f032d93f642f64180aa3";
const EVM_DESTINATION: &str = "70997970c51812dc3a010c7d01b50e0d17dc79c8";
const LOCK_NONCE: u64 = 1;
const LOCK_AMOUNT: u64 = 12_345;
const LOCK_FEE: u64 = 0;

fn funds() -> KeyPair {
    KeyPair::from(PrivateKey::from_str(REWARD_KEY).unwrap())
}

fn chain_config() -> ChainConfig {
    ChainConfig {
        chain_id: 1,
        hash_function: AnyHash::Blake2b(AnyHash32::default()),
        address_format: AddressFormat::Nimiq,
        endianness: Endianness::LittleEndian,
        block_time: Duration::from_secs(60),
        validation_program: ValidationProgram::new(vec![
            ValidationOp::PushConst(0),
            ValidationOp::LoadAddress,
            ValidationOp::Store("target_address".to_string()),
        ]),
        max_proof_depth: 64,
    }
}

/// The lock data: nonce (u64 BE) ++ EVM destination ++ EVM chain id (u64 BE) ++ EVM bridge.
fn lock_data() -> Vec<u8> {
    let mut data = Vec::with_capacity(56);
    data.extend_from_slice(&LOCK_NONCE.to_be_bytes());
    data.extend_from_slice(&hex::decode(EVM_DESTINATION).unwrap());
    data.extend_from_slice(&EVM_CHAIN_ID.to_be_bytes());
    data.extend_from_slice(&hex::decode(EVM_BRIDGE).unwrap());
    data
}

fn lock_tx(bridge: &Address, validity_start_height: u32) -> Transaction {
    let sender = funds();
    let mut tx = Transaction::new_extended(
        Address::from(&sender.public),
        AccountType::Basic,
        vec![],
        bridge.clone(),
        AccountType::Bridge,
        lock_data(),
        Coin::from_u64_unchecked(LOCK_AMOUNT),
        Coin::from_u64_unchecked(LOCK_FEE),
        validity_start_height,
        NetworkId::UnitAlbatross,
    );
    tx.proof = SignatureProof::from_ed25519(sender.public, sender.sign(&tx.serialize_content()))
        .serialize_to_vec();
    tx
}

/// Produces and pushes the next block with a seeded RNG, so the chain, and the vector written
/// from it, are the same on every run.
fn push_block(
    node: &TemporaryBlockProducer,
    transactions: Vec<Transaction>,
    rng: &mut StdRng,
) -> Block {
    let blockchain = node.blockchain.upgradable_read();
    let timestamp = blockchain.timestamp() + Policy::BLOCK_SEPARATION_TIME;
    let block = if Policy::is_macro_block_at(blockchain.block_number() + 1) {
        let proposal = node
            .producer
            .next_macro_block_proposal_with_rng(&blockchain, timestamp, 0, vec![], None, rng)
            .unwrap();
        Block::Macro(sign_macro_block(
            &node.producer.voting_key,
            proposal.header,
            proposal.body,
        ))
    } else {
        Block::Micro(
            node.producer
                .next_micro_block_with_rng(
                    &blockchain,
                    timestamp,
                    vec![],
                    transactions,
                    vec![],
                    None,
                    rng,
                )
                .unwrap(),
        )
    };
    assert_eq!(
        Blockchain::push(blockchain, block.clone()),
        Ok(PushResult::Extended)
    );
    block
}

fn hex0x(bytes: &[u8]) -> String {
    format!("0x{}", hex::encode(bytes))
}

/// Proves a lock to a bridge account through the RPC against two checkpoint blocks and the
/// election block of its epoch. With `NIMIQ_RPC_VECTOR_OUT` set, writes the RPC responses as
/// JSON to that path, as the EVM verifier's RPC-sourced test vector.
#[test(tokio::test)]
async fn history_proof_of_a_lock_as_an_evm_vector() {
    let node =
        TemporaryBlockProducer::new_with_protocol_version(upgrades::v3::BRIDGE_ORACLE_CONTRACTS);
    let dispatcher = BlockchainDispatcher::new(node.blockchain.clone().into());
    let height = || node.blockchain.read().block_number();
    let mut rng = test_rng(true);

    let owner = funds();
    let create_oracle = TransactionBuilder::new_create_oracle(
        &funds(),
        Address::from(&owner.public),
        4,
        Coin::from_u64_unchecked(1_000),
        Coin::ZERO,
        height(),
        NetworkId::UnitAlbatross,
    )
    .unwrap();
    let create_bridge = TransactionBuilder::new_create_bridge(
        &funds(),
        Address::from(&owner.public),
        create_oracle.recipient.clone(),
        1,
        chain_config(),
        Coin::from_u64_unchecked(10_000),
        Coin::ZERO,
        height(),
        NetworkId::UnitAlbatross,
    )
    .unwrap();
    let bridge = create_bridge.recipient.clone();
    push_block(&node, vec![create_oracle], &mut rng);
    push_block(&node, vec![create_bridge], &mut rng);

    let lock = lock_tx(&bridge, height());
    let lock_hash: Blake2bHash = lock.hash();
    let lock_block = push_block(&node, vec![lock.clone()], &mut rng).block_number();

    let election = Policy::election_block_after(lock_block);
    while height() < election {
        push_block(&node, vec![], &mut rng);
    }

    let first_checkpoint = Policy::macro_block_after(lock_block);
    let mut proofs = vec![];
    for macro_number in [
        first_checkpoint,
        first_checkpoint + Policy::blocks_per_batch(),
        election,
    ] {
        let block = node
            .blockchain
            .read()
            .chain_store
            .get_block_at(macro_number, false, None)
            .unwrap();
        let proof = dispatcher
            .get_transaction_history_proof(lock_hash.clone(), macro_number)
            .await
            .unwrap()
            .data;
        assert_proof_verifies(&proof, &lock_hash, &block);

        let raw_header = dispatcher
            .get_raw_macro_header(macro_number)
            .await
            .unwrap()
            .data;
        assert_eq!(raw_header.len(), 2 * 371);

        proofs.push(json!({
            "macroBlockNumber": macro_number,
            "blockHash": hex0x(block.hash().as_bytes()),
            "rawMacroHeader": format!("0x{raw_header}"),
            "response": serde_json::to_value(&proof).unwrap(),
        }));
    }

    // The proofs against the two checkpoints are against earlier states of the tree.
    let sizes: Vec<_> = proofs
        .iter()
        .map(|p| p["response"]["mmrSize"].as_u64().unwrap())
        .collect();
    assert!(sizes[0] < sizes[1] && sizes[1] < sizes[2]);

    let Some(path) = std::env::var_os("NIMIQ_RPC_VECTOR_OUT") else {
        return;
    };
    let block_time = node
        .blockchain
        .read()
        .chain_store
        .get_block_at(lock_block, false, None)
        .unwrap()
        .timestamp();
    let vector = json!({
        "source": "core-rs rpc-server/tests/history_proof.rs, history_proof_of_a_lock_as_an_evm_vector",
        "networkId": NetworkId::UnitAlbatross as u8,
        "nimiqBridge": hex0x(bridge.as_bytes()),
        "lock": {
            "transactionHash": hex0x(lock_hash.as_bytes()),
            "expected": {
                "blockNumber": lock_block,
                "blockTime": block_time,
                "sender": hex0x(lock.sender.as_bytes()),
                "recipient": hex0x(bridge.as_bytes()),
                "value": LOCK_AMOUNT,
                "fee": LOCK_FEE,
                "validityStartHeight": lock.validity_start_height,
                "networkId": NetworkId::UnitAlbatross as u8,
                "nonce": LOCK_NONCE,
                "destination": format!("0x{EVM_DESTINATION}"),
                "evmChainId": EVM_CHAIN_ID,
                "evmBridge": format!("0x{EVM_BRIDGE}"),
            },
        },
        "proofs": proofs,
    });
    std::fs::write(path, serde_json::to_string_pretty(&vector).unwrap() + "\n").unwrap();
}
