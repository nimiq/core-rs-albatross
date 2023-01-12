use nimiq_account::StakingContract;
use nimiq_transaction::ExecutedTransaction;
use parking_lot::RwLock;
use std::convert::TryInto;
use std::sync::Arc;
use tempfile::tempdir;

use beserial::Deserialize;
use nimiq_block::{Block, ForkProof, MicroJustification};
use nimiq_block_production::BlockProducer;

use nimiq_blockchain::{Blockchain, BlockchainConfig};
use nimiq_blockchain_interface::{AbstractBlockchain, PushResult};
use nimiq_database::{mdbx::MdbxEnvironment, volatile::VolatileEnvironment};
use nimiq_genesis::NetworkId;
use nimiq_hash::{Blake2bHash, Hash};
use nimiq_keys::{
    Address, KeyPair as SchnorrKeyPair, PrivateKey as SchnorrPrivateKey, SecureGenerate,
};
use nimiq_primitives::coin::Coin;
use nimiq_primitives::policy::Policy;
use nimiq_test_log::test;
use nimiq_test_utils::blockchain::{
    fill_micro_blocks, fill_micro_blocks_with_txns, sign_macro_block, signing_key, voting_key,
};
use nimiq_transaction_builder::TransactionBuilder;
use nimiq_trie::key_nibbles::KeyNibbles;
use nimiq_utils::time::OffsetTime;

const ADDRESS: &str = "NQ20TSB0DFSMUH9C15GQGAGJTTE4D3MA859E";

pub const ACCOUNT_SECRET_KEY: &str =
    "6c9320ac201caf1f8eaa5b05f5d67a9e77826f3f6be266a0ecccc20416dc6587";
pub const VALIDATOR_SIGNING_KEY: &str =
    "041580cc67e66e9e08b68fd9e4c9deb68737168fbe7488de2638c2e906c2f5ad";
const STAKER_ADDRESS: &str = "NQ20TSB0DFSMUH9C15GQGAGJTTE4D3MA859E";
pub const VALIDATOR_SECRET_KEY: &str =
    "6927eb8de74e8ea06a8afae5a66db176a7031f742b656651ac53bddb8a4ad3f3";
const VOLATILE_ENV: bool = true;

#[test]
fn it_can_produce_micro_blocks() {
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
    let producer = BlockProducer::new(signing_key(), voting_key());

    let bc = blockchain.upgradable_read();

    // Store seed before pushing a block as it is needed for the fork proof.
    let prev_vrf_seed = bc.head().seed().clone();

    // #1.0: Empty standard micro block
    let block = producer.next_micro_block(
        &bc,
        bc.head().timestamp() + Policy::BLOCK_SEPARATION_TIME,
        vec![],
        vec![],
        vec![0x41],
        None,
    );

    assert_eq!(
        Blockchain::push(bc, Block::Micro(block.clone())),
        Ok(PushResult::Extended)
    );

    assert_eq!(blockchain.read().block_number(), 1);

    // Create fork at #1.0
    let fork_proof = {
        let header1 = block.header.clone();
        let justification1 = match block.justification.unwrap() {
            MicroJustification::Micro(justification) => justification,
            MicroJustification::Skip(_) => {
                unreachable!("Block must not contain a skip block proof")
            }
        };
        let mut header2 = header1.clone();
        header2.timestamp += 1;
        let hash2 = header2.hash::<Blake2bHash>();
        let justification2 = signing_key().sign(hash2.as_slice());
        ForkProof {
            header1,
            header2,
            justification1,
            justification2,
            prev_vrf_seed,
        }
    };

    let bc = blockchain.upgradable_read();
    // #2.0: Empty micro block with fork proof
    let block = producer.next_micro_block(
        &bc,
        bc.head().timestamp() + Policy::BLOCK_SEPARATION_TIME,
        vec![fork_proof],
        vec![],
        vec![0x41],
        None,
    );
    assert_eq!(
        Blockchain::push(bc, Block::Micro(block)),
        Ok(PushResult::Extended)
    );

    assert_eq!(blockchain.read().block_number(), 2);

    // #2.1: Empty micro block (wrong prev_hash)
    let bc = blockchain.upgradable_read();
    let block = producer.next_micro_block(
        &bc,
        bc.head().timestamp() + Policy::BLOCK_SEPARATION_TIME,
        vec![],
        vec![],
        vec![0x41],
        None,
    );

    // the block justification is ok.
    assert_eq!(
        Blockchain::push(bc, Block::Micro(block)),
        Ok(PushResult::Extended)
    );
    assert_eq!(blockchain.read().block_number(), 3);

    // #2.2: Empty micro block
    let bc = blockchain.upgradable_read();
    let block = producer.next_micro_block(
        &bc,
        bc.head().timestamp() + Policy::BLOCK_SEPARATION_TIME,
        vec![],
        vec![],
        vec![0x41],
        None,
    );
    assert_eq!(
        Blockchain::push(bc, Block::Micro(block)),
        Ok(PushResult::Extended)
    );
    assert_eq!(blockchain.read().block_number(), 4);
}

#[test]
fn it_can_produce_macro_blocks() {
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
    let producer = BlockProducer::new(signing_key(), voting_key());

    fill_micro_blocks(&producer, &blockchain);

    let bc = blockchain.upgradable_read();
    let macro_block = {
        producer.next_macro_block_proposal(
            &bc,
            bc.head().timestamp() + Policy::BLOCK_SEPARATION_TIME,
            0u32,
            vec![],
        )
    };

    let block = sign_macro_block(&voting_key(), macro_block.header, macro_block.body);
    assert_eq!(
        Blockchain::push(bc, Block::Macro(block)),
        Ok(PushResult::Extended)
    );
}

#[test]
fn it_can_produce_election_blocks() {
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
    let producer = BlockProducer::new(signing_key(), voting_key());

    // push micro and macro blocks until the 3rd epoch is reached
    while Policy::epoch_at(blockchain.read().block_number()) < 2 {
        fill_micro_blocks(&producer, &blockchain);

        let bc = blockchain.upgradable_read();
        let macro_block = {
            producer.next_macro_block_proposal(
                &bc,
                bc.head().timestamp() + Policy::BLOCK_SEPARATION_TIME,
                0u32,
                vec![0x42],
            )
        };

        let block = sign_macro_block(&voting_key(), macro_block.header, macro_block.body);

        assert_eq!(
            Blockchain::push(bc, Block::Macro(block)),
            Ok(PushResult::Extended)
        );
    }
}

#[test]
fn it_can_produce_a_chain_with_txns() {
    let time = Arc::new(OffsetTime::new());
    let env = if VOLATILE_ENV {
        VolatileEnvironment::new(10).unwrap()
    } else {
        let tmp_dir = tempdir().expect("Could not create temporal directory");
        let tmp_dir = tmp_dir.path().to_str().unwrap();
        MdbxEnvironment::new(tmp_dir, 1024 * 1024 * 1024 * 1024, 21).unwrap()
    };
    let blockchain = Arc::new(RwLock::new(
        Blockchain::new(
            env,
            BlockchainConfig::default(),
            NetworkId::UnitAlbatross,
            time,
        )
        .unwrap(),
    ));
    let producer = BlockProducer::new(signing_key(), voting_key());

    // Small chain, otherwise test takes too long, use a small number of txns when running in volatile env
    // This test was intended to be used with an infinite loop and a high number of transactions per block though
    for _ in 0..1 {
        fill_micro_blocks_with_txns(&producer, &blockchain, 5, 0);

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

#[test]
fn it_can_revert_unpark_transactions() {
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
    let producer = BlockProducer::new(signing_key(), voting_key());

    // #1.0: Empty micro block
    let bc = blockchain.upgradable_read();

    let block = producer.next_micro_block(
        &bc,
        bc.head().timestamp() + Policy::BLOCK_SEPARATION_TIME,
        vec![],
        vec![],
        vec![0x41],
        None,
    );

    assert_eq!(
        Blockchain::push(bc, Block::Micro(block)),
        Ok(PushResult::Extended)
    );

    assert_eq!(blockchain.read().block_number(), 1);

    let bc = blockchain.upgradable_read();

    // One empty block
    let block = producer.next_micro_block(
        &bc,
        bc.head().timestamp() + Policy::BLOCK_SEPARATION_TIME,
        vec![],
        vec![],
        vec![0x41],
        None,
    );

    assert_eq!(
        Blockchain::push(bc, Block::Micro(block)),
        Ok(PushResult::Extended)
    );

    assert_eq!(blockchain.read().block_number(), 2);

    // One block with stacking transactions

    let mut transactions = vec![];
    let key_pair = signing_key();
    let address = Address::from_any_str(ADDRESS).unwrap();

    let tx = TransactionBuilder::new_unpark_validator(
        &key_pair,
        address,
        &key_pair,
        Coin::ZERO,
        1,
        NetworkId::UnitAlbatross,
    )
    .unwrap();

    transactions.push(tx);

    let bc = blockchain.upgradable_read();

    // Block with stacking transactions
    let block = producer.next_micro_block(
        &bc,
        bc.head().timestamp() + Policy::BLOCK_SEPARATION_TIME,
        vec![],
        transactions,
        vec![0x41],
        None,
    );

    assert_eq!(
        Blockchain::push(bc, Block::Micro(block)),
        Ok(PushResult::Extended)
    );

    assert_eq!(blockchain.read().block_number(), 3);

    let bc = blockchain.upgradable_read();

    let mut txn = bc.write_transaction();

    let result = bc.revert_blocks(3, &mut txn);

    assert_eq!(result, Ok(()));
}

#[test]
fn it_can_revert_create_staker_transaction() {
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
    let producer = BlockProducer::new(signing_key(), voting_key());

    // #1.0: Empty micro block
    let bc = blockchain.upgradable_read();

    let block = producer.next_micro_block(
        &bc,
        bc.head().timestamp() + Policy::BLOCK_SEPARATION_TIME,
        vec![],
        vec![],
        vec![0x41],
        None,
    );
    assert_eq!(
        Blockchain::push(bc, Block::Micro(block)),
        Ok(PushResult::Extended)
    );
    assert_eq!(blockchain.read().block_number(), 1);

    let bc = blockchain.upgradable_read();

    // One empty block
    let block = producer.next_micro_block(
        &bc,
        bc.head().timestamp() + Policy::BLOCK_SEPARATION_TIME,
        vec![],
        vec![],
        vec![0x41],
        None,
    );

    assert_eq!(
        Blockchain::push(bc, Block::Micro(block)),
        Ok(PushResult::Extended)
    );

    assert_eq!(blockchain.read().block_number(), 2);

    // One block with stacking transactions

    let mut transactions = vec![];
    let key_pair = ed25519_key_pair(ACCOUNT_SECRET_KEY);
    let address = Address::from_any_str(STAKER_ADDRESS).unwrap();

    let tx = TransactionBuilder::new_create_staker(
        &key_pair,
        &key_pair,
        Some(address),
        100_000_000.try_into().unwrap(),
        100.try_into().unwrap(),
        1,
        NetworkId::UnitAlbatross,
    )
    .unwrap();

    transactions.push(tx);

    let bc = blockchain.upgradable_read();

    // Block with stacking transactions
    let block = producer.next_micro_block(
        &bc,
        bc.head().timestamp() + Policy::BLOCK_SEPARATION_TIME,
        vec![],
        transactions,
        vec![0x41],
        None,
    );

    assert_eq!(
        Blockchain::push(bc, Block::Micro(block)),
        Ok(PushResult::Extended)
    );

    assert_eq!(blockchain.read().block_number(), 3);

    let bc = blockchain.upgradable_read();

    let mut txn = bc.write_transaction();
    let result = bc.revert_blocks(3, &mut txn);

    assert_eq!(result, Ok(()));
}

#[test]
fn it_can_revert_failed_transactions() {
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
    let producer = BlockProducer::new(signing_key(), voting_key());

    // These values will be used at the end of the test
    let initial_root = blockchain
        .read()
        .state()
        .accounts
        .get_root_hash_assert(None);
    let initial_history = blockchain
        .read()
        .history_store
        .get_history_tree_root(0, None);

    let bc = blockchain.upgradable_read();

    let block = producer.next_micro_block(
        &bc,
        bc.head().timestamp() + Policy::BLOCK_SEPARATION_TIME,
        vec![],
        vec![],
        vec![0x41],
        None,
    );
    assert_eq!(
        Blockchain::push(bc, Block::Micro(block)),
        Ok(PushResult::Extended)
    );
    assert_eq!(blockchain.read().block_number(), 1);

    let bc = blockchain.upgradable_read();

    // One empty block
    let block = producer.next_micro_block(
        &bc,
        bc.head().timestamp() + Policy::BLOCK_SEPARATION_TIME,
        vec![],
        vec![],
        vec![0x41],
        None,
    );

    assert_eq!(
        Blockchain::push(bc, Block::Micro(block)),
        Ok(PushResult::Extended)
    );

    assert_eq!(blockchain.read().block_number(), 2);

    // One block with stacking transactions

    let mut transactions = vec![];
    let key_pair = ed25519_key_pair(ACCOUNT_SECRET_KEY);
    let address = Address::from_any_str(STAKER_ADDRESS).unwrap();

    let tx_valid = TransactionBuilder::new_create_staker(
        &key_pair,
        &key_pair.clone(),
        Some(address.clone()),
        100_000_000.try_into().unwrap(),
        200.try_into().unwrap(),
        1,
        NetworkId::UnitAlbatross,
    )
    .unwrap();

    transactions.push(tx_valid.clone());

    // We will send a second create staker transaction, this should fail, as the same staker cannot be created twice
    let tx_failed = TransactionBuilder::new_create_staker(
        &key_pair,
        &key_pair,
        Some(address),
        100_000_000.try_into().unwrap(),
        100.try_into().unwrap(),
        1,
        NetworkId::UnitAlbatross,
    )
    .unwrap();

    transactions.push(tx_failed.clone());

    let bc = blockchain.upgradable_read();

    // Block with stacking transactions
    let block = producer.next_micro_block(
        &bc,
        bc.head().timestamp() + Policy::BLOCK_SEPARATION_TIME,
        vec![],
        transactions,
        vec![0x41],
        None,
    );

    assert_eq!(
        Blockchain::push(bc, Block::Micro(block.clone())),
        Ok(PushResult::Extended)
    );

    let block_transactions = block.body.unwrap().transactions;

    assert_eq!(block_transactions[0], ExecutedTransaction::Ok(tx_valid));
    assert_eq!(block_transactions[1], ExecutedTransaction::Err(tx_failed));

    let bc = &blockchain.read();

    let accounts = &bc.state.accounts;

    // We need to check that the fee was deducted from the sender account for both transactions
    assert_eq!(
        accounts
            .get(&KeyNibbles::from(&Address::from(&key_pair.public)), None)
            .unwrap()
            .balance(),
        Coin::from_u64_unchecked(999899999700)
    );

    assert_eq!(bc.block_number(), 3);

    let mut txn = bc.write_transaction();
    let result = bc.revert_blocks(3, &mut txn);

    txn.commit();

    let final_root = bc.state().accounts.get_root_hash_assert(None);
    let final_history = bc.history_store.get_history_tree_root(0, None);

    // Verify that the state after reverting the blocks is equal to the initial state.
    assert_eq!(initial_root, final_root);
    assert_eq!(initial_history, final_history);

    assert!(result.is_ok());
}

#[test]
fn it_can_revert_failed_vesting_contract_transaction() {
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
    let producer = BlockProducer::new(signing_key(), voting_key());

    // #1.0: Micro block that creates a vesting contract
    let bc = blockchain.upgradable_read();

    let key_pair = ed25519_key_pair(ACCOUNT_SECRET_KEY);
    let address = Address::from_any_str(STAKER_ADDRESS).unwrap();

    let tx = TransactionBuilder::new_create_vesting(
        &key_pair,
        Address::from(&key_pair.public),
        1,
        1,
        1,
        Coin::from_u64_unchecked(1000),
        Coin::from_u64_unchecked(100),
        1,
        NetworkId::UnitAlbatross,
    )
    .unwrap();

    let transactions = vec![tx.clone()];

    let block = producer.next_micro_block(
        &bc,
        bc.head().timestamp() + Policy::BLOCK_SEPARATION_TIME,
        vec![],
        transactions,
        vec![0x41],
        None,
    );
    assert_eq!(
        Blockchain::push(bc, Block::Micro(block)),
        Ok(PushResult::Extended)
    );
    assert_eq!(blockchain.read().block_number(), 1);

    // Now we need to verify the contract was created and it has the right balance
    let bc = blockchain.read();
    let accounts = &bc.state.accounts;

    assert_eq!(
        accounts
            .get(&KeyNibbles::from(&tx.contract_creation_address()), None)
            .unwrap()
            .balance(),
        Coin::from_u64_unchecked(1000)
    );

    // We verify the value + fee was properly deducted from the accounts balance
    assert_eq!(
        accounts
            .get(&KeyNibbles::from(&Address::from(&key_pair.public)), None)
            .unwrap()
            .balance(),
        Coin::from_u64_unchecked(999999998900)
    );

    drop(bc);

    // These values will be used at the end of the test
    let initial_root = blockchain
        .read()
        .state()
        .accounts
        .get_root_hash_assert(None);
    let initial_history = blockchain
        .read()
        .history_store
        .get_history_tree_root(0, None);

    let bc = blockchain.upgradable_read();

    //Now we create a redeem funds transaction that will fail
    let redeem_tx = TransactionBuilder::new_redeem_vesting(
        &key_pair,
        tx.contract_creation_address(),
        address,
        Coin::from_u64_unchecked(2000),
        Coin::from_u64_unchecked(100),
        1,
        NetworkId::UnitAlbatross,
    )
    .unwrap();

    let transactions = vec![redeem_tx.clone()];

    // Block with redeem funds transaction
    let block = producer.next_micro_block(
        &bc,
        bc.head().timestamp() + Policy::BLOCK_SEPARATION_TIME,
        vec![],
        transactions,
        vec![0x41],
        None,
    );

    assert_eq!(
        Blockchain::push(bc, Block::Micro(block.clone())),
        Ok(PushResult::Extended)
    );

    assert_eq!(blockchain.read().block_number(), 2);

    let block_transactions = block.body.unwrap().transactions;

    assert_eq!(block_transactions[0], ExecutedTransaction::Err(redeem_tx));

    let bc = &blockchain.read();

    let accounts = &bc.state.accounts;

    // We need to check that the fee was deducted from the contract balance for the failed transaction
    assert_eq!(
        accounts
            .get(&KeyNibbles::from(&tx.contract_creation_address()), None)
            .unwrap()
            .balance(),
        Coin::from_u64_unchecked(900)
    );

    // The sender balance should not have changed
    assert_eq!(
        accounts
            .get(&KeyNibbles::from(&Address::from(&key_pair.public)), None)
            .unwrap()
            .balance(),
        Coin::from_u64_unchecked(999999998900)
    );

    let mut txn = bc.write_transaction();
    let result = bc.revert_blocks(1, &mut txn);

    txn.commit();

    // We need to check that the contract balance has the initial funds after reverting the block
    assert_eq!(
        accounts
            .get(&KeyNibbles::from(&tx.contract_creation_address()), None)
            .unwrap()
            .balance(),
        Coin::from_u64_unchecked(1000)
    );

    let final_root = bc.state().accounts.get_root_hash_assert(None);
    let final_history = bc.history_store.get_history_tree_root(0, None);

    // Verify that the state after reverting the blocks is equal to the initial state.
    assert_eq!(initial_root, final_root);
    assert_eq!(initial_history, final_history);

    assert!(result.is_ok());
}

#[test]
fn it_can_revert_reactivate_transaction() {
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
    let producer = BlockProducer::new(signing_key(), voting_key());

    // One block with stacking transactions
    let mut transactions = vec![];
    let key_pair = ed25519_key_pair(ACCOUNT_SECRET_KEY);
    let address =
        Address::from_user_friendly_address("NQ20 TSB0 DFSM UH9C 15GQ GAGJ TTE4 D3MA 859E")
            .unwrap();
    let signing_key_pair = ed25519_key_pair(VALIDATOR_SIGNING_KEY);

    let tx = TransactionBuilder::new_inactivate_validator(
        &key_pair,
        address.clone(),
        &signing_key_pair,
        100.try_into().unwrap(),
        1,
        NetworkId::UnitAlbatross,
    )
    .unwrap();

    transactions.push(tx.clone());

    let bc = blockchain.upgradable_read();

    // Block with stacking transactions
    let block = producer.next_micro_block(
        &bc,
        bc.head().timestamp() + Policy::BLOCK_SEPARATION_TIME,
        vec![],
        transactions,
        vec![0x41],
        None,
    );

    let block_transactions = &block.body.as_ref().unwrap().transactions;

    assert_eq!(block_transactions[0], ExecutedTransaction::Ok(tx));

    assert_eq!(
        Blockchain::push(bc, Block::Micro(block)),
        Ok(PushResult::Extended)
    );

    assert_eq!(blockchain.read().block_number(), 1);

    //Now create the reactivate transaction

    let mut transactions = vec![];
    let tx = TransactionBuilder::new_reactivate_validator(
        &key_pair,
        address,
        &signing_key_pair,
        100.try_into().unwrap(),
        1,
        NetworkId::UnitAlbatross,
    )
    .unwrap();

    transactions.push(tx);

    let bc = blockchain.upgradable_read();

    // Block with stacking transactions
    let block = producer.next_micro_block(
        &bc,
        bc.head().timestamp() + Policy::BLOCK_SEPARATION_TIME,
        vec![],
        transactions,
        vec![0x41],
        None,
    );

    assert_eq!(
        Blockchain::push(bc, Block::Micro(block)),
        Ok(PushResult::Extended)
    );

    assert_eq!(blockchain.read().block_number(), 2);

    let bc = blockchain.upgradable_read();

    let mut txn = bc.write_transaction();
    // Revert the reactivate transaction
    let result = bc.revert_blocks(2, &mut txn);

    assert!(result.is_ok());
}

#[test]
fn it_can_revert_unpark_transaction() {
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
    let producer = BlockProducer::new(signing_key(), voting_key());

    // One block with stacking transactions
    let mut transactions = vec![];
    let key_pair = ed25519_key_pair(ACCOUNT_SECRET_KEY);
    let address =
        Address::from_user_friendly_address("NQ20 TSB0 DFSM UH9C 15GQ GAGJ TTE4 D3MA 859E")
            .unwrap();
    let signing_key_pair = ed25519_key_pair(VALIDATOR_SIGNING_KEY);

    let tx = TransactionBuilder::new_unpark_validator(
        &key_pair,
        address.clone(),
        &signing_key_pair,
        100.try_into().unwrap(),
        1,
        NetworkId::UnitAlbatross,
    )
    .unwrap();

    transactions.push(tx);

    let bc = blockchain.upgradable_read();

    // Block with stacking transactions
    let block = producer.next_micro_block(
        &bc,
        bc.head().timestamp() + Policy::BLOCK_SEPARATION_TIME,
        vec![],
        transactions,
        vec![0x41],
        None,
    );

    assert_eq!(
        Blockchain::push(bc, Block::Micro(block)),
        Ok(PushResult::Extended)
    );

    assert_eq!(blockchain.read().block_number(), 1);

    //Now create the reactivate transaction

    let mut transactions = vec![];
    let tx = TransactionBuilder::new_reactivate_validator(
        &key_pair,
        address,
        &signing_key_pair,
        10.try_into().unwrap(),
        1,
        NetworkId::UnitAlbatross,
    )
    .unwrap();

    transactions.push(tx);

    let bc = blockchain.upgradable_read();

    // Block with stacking transactions
    let block = producer.next_micro_block(
        &bc,
        bc.head().timestamp() + Policy::BLOCK_SEPARATION_TIME,
        vec![],
        transactions,
        vec![0x41],
        None,
    );

    assert_eq!(
        Blockchain::push(bc, Block::Micro(block)),
        Ok(PushResult::Extended)
    );

    assert_eq!(blockchain.read().block_number(), 2);

    let bc = blockchain.upgradable_read();

    let mut txn = bc.write_transaction();
    // Revert the reactivate transaction
    let result = bc.revert_blocks(2, &mut txn);

    assert!(result.is_ok());
}

#[test]
fn it_can_consume_all_validator_deposit() {
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
    let producer = BlockProducer::new(signing_key(), voting_key());

    // One block with stacking transactions
    let mut transactions = vec![];
    let key_pair = ed25519_key_pair(VALIDATOR_SECRET_KEY);
    let address =
        Address::from_user_friendly_address("NQ20 TSB0 DFSM UH9C 15GQ GAGJ TTE4 D3MA 859E")
            .unwrap();

    let signing_key_pair = ed25519_key_pair(VALIDATOR_SIGNING_KEY);
    let account_key_pair = ed25519_key_pair(ACCOUNT_SECRET_KEY);
    let tx = TransactionBuilder::new_inactivate_validator(
        &account_key_pair,
        Address::from(&key_pair.public),
        &signing_key_pair,
        Coin::from_u64_unchecked(100),
        1,
        NetworkId::UnitAlbatross,
    )
    .unwrap();

    transactions.push(tx);

    // This is an invalid tx since it will try to move more than the validator deposit
    let invalid_tx = TransactionBuilder::new_delete_validator(
        address.clone(),
        &key_pair,
        Coin::from_u64_unchecked(100),
        Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT),
        1,
        NetworkId::UnitAlbatross,
    )
    .unwrap();

    transactions.push(invalid_tx.clone());

    let bc = blockchain.upgradable_read();

    // Block with stacking transactions
    let block = producer.next_micro_block(
        &bc,
        bc.head().timestamp() + Policy::BLOCK_SEPARATION_TIME,
        vec![],
        transactions,
        vec![0x41],
        None,
    );

    let block_transactions = &block.body.as_ref().unwrap().transactions;

    assert_eq!(block_transactions[1], ExecutedTransaction::Err(invalid_tx));

    assert_eq!(
        Blockchain::push(bc, Block::Micro(block)),
        Ok(PushResult::Extended)
    );

    assert_eq!(blockchain.read().block_number(), 1);

    // Now we need to verify that the validator deposit was reduced because of the failing txn
    {
        let blockchain = blockchain.read();
        let accounts_tree = &blockchain.state().accounts.tree;
        let db_txn = blockchain.read_transaction();
        let validator = StakingContract::get_validator(
            accounts_tree,
            &db_txn,
            &Address::from(&key_pair.public),
        )
        .unwrap()
        .unwrap();

        assert_eq!(
            validator.deposit,
            Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT - 100)
        );
    }

    //Send another transaction that will consume all the remaining validator deposit, and thus, the validator is deleted
    let mut transactions = vec![];

    let invalid_tx = TransactionBuilder::new_delete_validator(
        address.clone(),
        &key_pair,
        Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT - 100),
        Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT),
        1,
        NetworkId::UnitAlbatross,
    )
    .unwrap();

    transactions.push(invalid_tx.clone());

    let bc = blockchain.upgradable_read();

    // Block with stacking transactions
    let block = producer.next_micro_block(
        &bc,
        bc.head().timestamp() + Policy::BLOCK_SEPARATION_TIME,
        vec![],
        transactions,
        vec![0x41],
        None,
    );

    let block_transactions = &block.body.as_ref().unwrap().transactions;

    assert_eq!(block_transactions[0], ExecutedTransaction::Err(invalid_tx));

    assert_eq!(
        Blockchain::push(bc, Block::Micro(block)),
        Ok(PushResult::Extended)
    );

    assert_eq!(blockchain.read().block_number(), 2);

    // Now we need to verify that the validator no longer exists
    {
        let blockchain = blockchain.read();
        let accounts_tree = &blockchain.state().accounts.tree;
        let db_txn = blockchain.read_transaction();

        // The validator should be deleted.
        let validator = StakingContract::get_validator(
            accounts_tree,
            &db_txn,
            &Address::from(&key_pair.public),
        )
        .unwrap();

        assert_eq!(validator, None);
    }
    let blockchain = blockchain.upgradable_read();
    let mut txn = blockchain.write_transaction();
    // Revert the failed delete transaction
    let result = blockchain.revert_blocks(1, &mut txn);

    assert!(result.is_ok());
    txn.commit();

    //Now the validator should be back:

    let accounts_tree = &blockchain.state().accounts.tree;
    let db_txn = blockchain.read_transaction();

    // The validator should be deleted.
    let validator =
        StakingContract::get_validator(accounts_tree, &db_txn, &Address::from(&key_pair.public))
            .unwrap();

    assert!(validator.is_some());
}

#[test]
fn it_can_revert_failed_delete_validator() {
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
    let producer = BlockProducer::new(signing_key(), voting_key());

    // One block with stacking transactions
    let mut transactions = vec![];
    let key_pair = ed25519_key_pair(VALIDATOR_SECRET_KEY);
    let address =
        Address::from_user_friendly_address("NQ20 TSB0 DFSM UH9C 15GQ GAGJ TTE4 D3MA 859E")
            .unwrap();

    // This is an invalid tx since it will try to move more than the validator deposit
    let invalid_tx = TransactionBuilder::new_delete_validator(
        address.clone(),
        &key_pair,
        Coin::from_u64_unchecked(100),
        Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT),
        1,
        NetworkId::UnitAlbatross,
    )
    .unwrap();

    transactions.push(invalid_tx.clone());

    let bc = blockchain.upgradable_read();

    // Block with stacking transactions
    let block = producer.next_micro_block(
        &bc,
        bc.head().timestamp() + Policy::BLOCK_SEPARATION_TIME,
        vec![],
        transactions,
        vec![0x41],
        None,
    );

    let block_transactions = &block.body.as_ref().unwrap().transactions;

    assert_eq!(block_transactions[0], ExecutedTransaction::Err(invalid_tx));

    assert_eq!(
        Blockchain::push(bc, Block::Micro(block)),
        Ok(PushResult::Extended)
    );

    assert_eq!(blockchain.read().block_number(), 1);

    // Now we need to verify that the validator deposit was reduced because of the failing txn
    {
        let blockchain = blockchain.read();
        let accounts_tree = &blockchain.state().accounts.tree;
        let db_txn = blockchain.read_transaction();
        let validator = StakingContract::get_validator(
            accounts_tree,
            &db_txn,
            &Address::from(&key_pair.public),
        )
        .unwrap()
        .unwrap();

        assert_eq!(
            validator.deposit,
            Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT - 100)
        );
        //Now the validator should be inactive because of the failing txn..
        assert_eq!(validator.inactivity_flag, Some(1));
    }

    let blockchain = blockchain.upgradable_read();

    let mut txn = blockchain.write_transaction();
    // Revert the reactivate transaction
    let result = blockchain.revert_blocks(1, &mut txn);

    assert!(result.is_ok());

    txn.commit();

    // Now we need to verify that the validator deposit was reduced because of the failing txn
    let accounts_tree = &blockchain.state().accounts.tree;
    let db_txn = blockchain.read_transaction();
    let validator =
        StakingContract::get_validator(accounts_tree, &db_txn, &Address::from(&key_pair.public))
            .unwrap()
            .unwrap();

    assert_eq!(
        validator.deposit,
        Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT)
    );
}

#[test]
fn it_can_revert_basic_and_create_contracts_txns() {
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
    let producer = BlockProducer::new(signing_key(), voting_key());

    // One block with stacking transactions
    let mut transactions = vec![];
    let key_pair = ed25519_key_pair(ACCOUNT_SECRET_KEY);

    let recipient_key_pair = SchnorrKeyPair::generate_default_csprng();
    let address = Address::from(&recipient_key_pair.public);

    let tx = TransactionBuilder::new_basic(
        &key_pair,
        address.clone(),
        100.try_into().unwrap(),
        Coin::ZERO,
        1,
        NetworkId::UnitAlbatross,
    )
    .unwrap();

    transactions.push(tx);

    let tx = TransactionBuilder::new_create_vesting(
        &key_pair,
        Address::from(&key_pair.public),
        1,
        1,
        1,
        Coin::from_u64_unchecked(1000),
        Coin::from_u64_unchecked(100),
        1,
        NetworkId::UnitAlbatross,
    )
    .unwrap();

    transactions.push(tx);

    let recipient_key_pair = SchnorrKeyPair::generate_default_csprng();
    let address = Address::from(&recipient_key_pair.public);

    let tx = TransactionBuilder::new_basic(
        &key_pair,
        address.clone(),
        100.try_into().unwrap(),
        Coin::ZERO,
        1,
        NetworkId::UnitAlbatross,
    )
    .unwrap();

    transactions.push(tx);

    let bc = blockchain.upgradable_read();

    // Block with txns
    let block = producer.next_micro_block(
        &bc,
        bc.head().timestamp() + Policy::BLOCK_SEPARATION_TIME,
        vec![],
        transactions,
        vec![0x41],
        None,
    );

    assert_eq!(
        Blockchain::push(bc, Block::Micro(block)),
        Ok(PushResult::Extended)
    );

    let bc = blockchain.upgradable_read();

    let mut txn = bc.write_transaction();
    // Revert the reactivate transaction
    let result = bc.revert_blocks(1, &mut txn);

    assert!(result.is_ok());
}

fn ed25519_key_pair(secret_key: &str) -> SchnorrKeyPair {
    let priv_key: SchnorrPrivateKey =
        Deserialize::deserialize(&mut &hex::decode(secret_key).unwrap()[..]).unwrap();
    priv_key.into()
}
