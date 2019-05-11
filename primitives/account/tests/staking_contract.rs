use std::collections::btree_set::BTreeSet;
use std::collections::HashMap;
use std::convert::{TryFrom, TryInto};
use std::sync::Arc;
use rand::thread_rng;

use beserial::{Deserialize, Serialize};
use nimiq_bls::bls12_381::KeyPair as BlsKeyPair;
use nimiq_keys::{Address, KeyPair};
use nimiq_primitives::coin::Coin;
use nimiq_primitives::networks::NetworkId;
use nimiq_account::{AccountError, AccountTransactionInteraction, AccountType, StakingContract};
use nimiq_transaction::{SignatureProof, Transaction, TransactionError};
use nimiq_transaction::account::AccountTransactionVerification;
use nimiq_transaction::account::staking_contract::StakingTransactionData;

const CONTRACT_1: &str = "00000000000000000000000000000000";
const CONTRACT_2: &str = "0000000023c34600000000020202020202020202020202020202020202020202000000001ad27480a2f7d485efe6fabad3d780d1ea5ad690bd027a5328f44b612cad1f33347c8df5bde90a340c30877a21861e2173f6cfda0715d35ac2941437bf7e73d7e48fcf6e1901249134532ad1826ad1e396caed2d4d1d11e82d79f93946b21800a00971f000005e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e0000000008f0d180a9edd1613b714ec6107f4ffd532e52727c4f3a2897b3000e9ebccf076e8ffdf4b424f7e798d31dc67bbf9b3776096f101740b3f992ba8a5d0e20860f8d3466b7b58fb6b918eebb3c014bf6bb1cbdcb045c184d673c3db6435f454a1c530b9dfc012a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a0000000000";

#[test]
fn it_can_de_serialize_a_staking_contract() {
    let bytes_1: Vec<u8> = hex::decode(CONTRACT_1).unwrap();
    let contract_1: StakingContract = Deserialize::deserialize(&mut &bytes_1[..]).unwrap();
    assert_eq!(contract_1.balance, 0.try_into().unwrap());
    assert_eq!(contract_1.active_stake_by_address.len(), 0);
    assert_eq!(contract_1.active_stake_sorted.len(), 0);
    assert_eq!(contract_1.inactive_stake_by_address.len(), 0);
    let mut bytes_1_out = Vec::<u8>::with_capacity(contract_1.serialized_size());
    let size_1_out = contract_1.serialize(&mut bytes_1_out).unwrap();
    assert_eq!(size_1_out, contract_1.serialized_size());
    assert_eq!(hex::encode(bytes_1_out), CONTRACT_1);

    let bytes_2: Vec<u8> = hex::decode(CONTRACT_2).unwrap();
    let contract_2: StakingContract = Deserialize::deserialize(&mut &bytes_2[..]).unwrap();
    assert_eq!(contract_2.balance, 600_000_000.try_into().unwrap());
    assert_eq!(contract_2.active_stake_by_address.len(), 2);
    assert_eq!(contract_2.active_stake_sorted.len(), 2);
    assert_eq!(contract_2.inactive_stake_by_address.len(), 0);
    let mut bytes_2_out = Vec::<u8>::with_capacity(contract_2.serialized_size());
    let size_2_out = contract_2.serialize(&mut bytes_2_out).unwrap();
    assert_eq!(size_2_out, contract_2.serialized_size());
    assert_eq!(hex::encode(bytes_2_out), CONTRACT_2);
}

#[test]
fn it_does_not_support_contract_creation() {
    let data: Vec<u8> = Vec::with_capacity(0);
    let sender = Address::from([3u8; 20]);
    let transaction = Transaction::new_contract_creation(
        data,
        sender.clone(),
        AccountType::Basic,
        AccountType::Staking,
        100.try_into().unwrap(),
        0.try_into().unwrap(),
        0,
        NetworkId::Dummy,
    );

    assert_eq!(AccountType::verify_incoming_transaction(&transaction), Err(TransactionError::InvalidForRecipient));
}

#[test]
fn it_can_verify_staking_transaction() {
    let bls_pair = BlsKeyPair::generate(&mut thread_rng());
    let mut tx = make_incoming_transaction();

    let proof_of_knowledge = bls_pair.sign(&bls_pair.public);

    let mut data = StakingTransactionData {
        validator_key: bls_pair.public.clone(),
        reward_address: Some(Address::from([3u8; 20])),
        proof_of_knowledge,
    };
    tx.data = data.serialize_to_vec();

    // Valid
    assert_eq!(AccountType::verify_incoming_transaction(&tx), Ok(()));

    // Below minimum stake
    tx.value = 123.try_into().unwrap();
    assert_eq!(AccountType::verify_incoming_transaction(&tx), Err(TransactionError::InvalidForRecipient));

    // Invalid proof of knowledge
    let other_pair = BlsKeyPair::generate(&mut thread_rng());
    let invalid_pok = other_pair.sign(&bls_pair.public);
    data.proof_of_knowledge = invalid_pok;
    tx.data = data.serialize_to_vec();
    assert_eq!(AccountType::verify_incoming_transaction(&tx), Err(TransactionError::InvalidData));
}

#[test]
fn it_can_apply_staking_transaction() {
    let mut contract = make_empty_contract();

    // Default transaction data
    let bls_pair = BlsKeyPair::generate(&mut thread_rng());
    let proof_of_knowledge = bls_pair.sign(&bls_pair.public);
    let stake_data = StakingTransactionData {
        validator_key: bls_pair.public.clone(),
        reward_address: None,
        proof_of_knowledge,
    };

    // Default stake
    let mut tx_1 = make_incoming_transaction();
    tx_1.data = stake_data.serialize_to_vec();
    assert_eq!(contract.check_incoming_transaction(&tx_1, 2), Ok(()));
    assert_eq!(contract.commit_incoming_transaction(&tx_1, 2), Ok(None));
    assert_eq!(contract.active_stake_by_address.len(), 1);
    assert_eq!(contract.balance, 150_000_000.try_into().unwrap());

    // Same stake again
    let mut tx_2 = make_incoming_transaction();
    tx_2.data = stake_data.serialize_to_vec();
    assert_eq!(contract.check_incoming_transaction(&tx_2, 3), Ok(()));
    let receipt_2 = contract.commit_incoming_transaction(&tx_2, 3).unwrap().unwrap();
    assert_eq!(contract.active_stake_by_address.len(), 1);
    assert_eq!(contract.balance, 300_000_000.try_into().unwrap());

    // Stake again, changing validator key
    let mut tx_3 = make_incoming_transaction();
    let bls_other = BlsKeyPair::generate(&mut thread_rng());
    let pok_other = bls_other.sign(&bls_other.public);
    tx_3.data = StakingTransactionData {
        validator_key: bls_other.public.clone(),
        reward_address: None,
        proof_of_knowledge: pok_other,
    }.serialize_to_vec();
    assert_eq!(contract.check_incoming_transaction(&tx_3, 4), Ok(()));
    let receipt_3 = contract.commit_incoming_transaction(&tx_3, 4).unwrap().unwrap();
    assert_eq!(contract.active_stake_by_address.len(), 1);
    assert_eq!(contract.balance, 450_000_000.try_into().unwrap());

    // Stake on new account with reward address
    let mut tx_4 = make_incoming_transaction();
    tx_4.sender = Address::from([94u8; 20]);
    let mut stake_data_4 = stake_data.clone();
    stake_data_4.reward_address = Some(Address::from([42u8; 20]));
    tx_4.data = stake_data_4.serialize_to_vec();
    assert_eq!(contract.check_incoming_transaction(&tx_4, 5), Ok(()));
    assert_eq!(contract.commit_incoming_transaction(&tx_4, 5), Ok(None));
    assert_eq!(contract.active_stake_by_address.len(), 2);
    assert_eq!(contract.balance, 600_000_000.try_into().unwrap());

    // Revert everything
    assert_eq!(contract.revert_incoming_transaction(&tx_4, 5, None), Ok(()));
    assert_eq!(contract.balance, 450_000_000.try_into().unwrap());
    assert_eq!(contract.revert_incoming_transaction(&tx_3, 4, Some(&receipt_3)), Ok(()));
    assert_eq!(contract.balance, 300_000_000.try_into().unwrap());
    assert_eq!(contract.revert_incoming_transaction(&tx_2, 3, Some(&receipt_2)), Ok(()));
    assert_eq!(contract.active_stake_by_address.len(), 1);
    assert_eq!(contract.balance, 150_000_000.try_into().unwrap());
    assert_eq!(contract.revert_incoming_transaction(&tx_1, 2, None), Ok(()));
    assert_eq!(contract.active_stake_by_address.len(), 0);
    assert_eq!(contract.balance, 0.try_into().unwrap());
}

fn test_proof_verification<F>(incoming: bool, make_tx: F) where F: Fn() -> Transaction {
    let key_pair = KeyPair::generate();
    let tx = make_tx();

    // No proof
    let tx_1 = tx.clone();
    assert!(AccountType::verify_outgoing_transaction(&tx_1).is_err());

    // Valid
    let mut tx_2 = tx.clone();
    tx_2.proof = SignatureProof::from(key_pair.public, key_pair.sign(&tx_2.serialize_content())).serialize_to_vec();
    assert_eq!(AccountType::verify_outgoing_transaction(&tx_2), Ok(()));
    if incoming {
        assert_eq!(AccountType::verify_incoming_transaction(&tx_2), Ok(()));
    }

    // Invalid proof
    let mut tx_3 = tx.clone();
    let other_pair = KeyPair::generate();
    tx_3.proof = SignatureProof::from(key_pair.public, other_pair.sign(&tx_3.serialize_content())).serialize_to_vec();
    assert_eq!(AccountType::verify_outgoing_transaction(&tx_3), Err(TransactionError::InvalidProof));
    if incoming {
        assert_eq!(AccountType::verify_incoming_transaction(&tx_3), Ok(()));
    }
}

#[test]
fn it_can_verify_retire_transaction() {
    let incoming = true;
    test_proof_verification(incoming, | | -> Transaction {
        let mut tx = make_outgoing_transaction();
        tx.recipient = tx.sender.clone();
        tx.recipient_type = AccountType::Staking;
        tx
    });
}

#[test]
fn it_can_apply_retiring_transaction() {
    let key_pair = KeyPair::generate();
    let bls_pair = BlsKeyPair::generate(&mut thread_rng());
    let mut contract = make_sample_contract(&key_pair, &bls_pair);

    // Retire first half of stake
    let mut tx_1 = make_outgoing_transaction();
    tx_1.recipient = tx_1.sender.clone();
    tx_1.proof = SignatureProof::from(key_pair.public, key_pair.sign(&tx_1.serialize_content())).serialize_to_vec();
    assert_eq!(contract.check_outgoing_transaction(&tx_1, 2), Ok(()));
    assert_eq!(contract.commit_outgoing_transaction(&tx_1, 2), Ok(None));
    assert_eq!(contract.check_incoming_transaction(&tx_1, 2), Ok(()));
    assert_eq!(contract.commit_incoming_transaction(&tx_1, 2).unwrap(), None);

    assert_eq!(contract.active_stake_by_address.len(), 1);
    assert_eq!(contract.active_stake_sorted.len(), 1);
    assert_eq!(contract.inactive_stake_by_address.len(), 1);
    assert_eq!(contract.balance, 299_999_766.try_into().unwrap());

    // Try to retire too much stake
    let mut tx_2 = make_outgoing_transaction();
    tx_2.value = 200_000_000.try_into().unwrap();
    tx_2.recipient = tx_2.sender.clone();
    tx_2.proof = SignatureProof::from(key_pair.public, key_pair.sign(&tx_2.serialize_content())).serialize_to_vec();
    let funds_error = AccountError::InsufficientFunds {
        needed:  200_000_234.try_into().unwrap(),
        balance: 150_000_000.try_into().unwrap(),
    };
    assert_eq!(contract.check_outgoing_transaction(&tx_2, 3), Err(funds_error.clone()));
    assert_eq!(contract.commit_outgoing_transaction(&tx_2, 3), Err(funds_error.clone()));

    // Retire second half of stake in two transactions
    let mut tx_3 = make_outgoing_transaction();
    tx_3.value = 74_999_766.try_into().unwrap();
    tx_3.recipient = tx_3.sender.clone();
    tx_3.proof = SignatureProof::from(key_pair.public, key_pair.sign(&tx_3.serialize_content())).serialize_to_vec();
    assert_eq!(contract.check_outgoing_transaction(&tx_3, 3), Ok(()));
    assert_eq!(contract.commit_outgoing_transaction(&tx_3, 3), Ok(None));
    assert_eq!(contract.check_outgoing_transaction(&tx_3, 3), Ok(()));
    let receipt_outgoing_2 = contract.commit_outgoing_transaction(&tx_3, 3).unwrap().unwrap();
    assert_eq!(contract.check_incoming_transaction(&tx_3, 3), Ok(()));
    let receipt_incoming_1 = contract.commit_incoming_transaction(&tx_3, 3).unwrap().unwrap();
    assert_eq!(contract.check_incoming_transaction(&tx_3, 3), Ok(()));
    let receipt_incoming_2 = contract.commit_incoming_transaction(&tx_3, 3).unwrap().unwrap();

    assert_eq!(contract.active_stake_by_address.len(), 0);
    assert_eq!(contract.active_stake_sorted.len(), 0);
    assert_eq!(contract.inactive_stake_by_address.len(), 1);
    assert_eq!(contract.balance, 299_999_298.try_into().unwrap());

    // Try to retire nonexistent funds
    assert_eq!(contract.check_outgoing_transaction(&tx_3, 4), Err(AccountError::InvalidForSender));
    assert_eq!(contract.commit_outgoing_transaction(&tx_3, 4), Err(AccountError::InvalidForSender));

    // Revert to original state
    assert_eq!(contract.revert_incoming_transaction(&tx_3, 3, Some(&receipt_incoming_2)), Ok(()));
    assert_eq!(contract.revert_incoming_transaction(&tx_3, 3, Some(&receipt_incoming_1)), Ok(()));
    assert_eq!(contract.revert_outgoing_transaction(&tx_3, 3, Some(&receipt_outgoing_2)), Ok(()));
    assert_eq!(contract.revert_outgoing_transaction(&tx_3, 3, None), Ok(()));
    assert_eq!(contract.revert_incoming_transaction(&tx_1, 2, None), Ok(()));
    assert_eq!(contract.revert_outgoing_transaction(&tx_1, 2, None), Ok(()));

    assert_eq!(contract.active_stake_by_address.len(), 1);
    assert_eq!(contract.active_stake_sorted.len(), 1);
    assert_eq!(contract.inactive_stake_by_address.len(), 0);
    assert_eq!(contract.balance, 300_000_000.try_into().unwrap());
}

#[test]
fn it_can_verify_unstaking_transaction() {
    let incoming = false;
    test_proof_verification(incoming, | | -> Transaction {
        let mut tx = make_outgoing_transaction();
        tx.recipient_type = AccountType::Staking;
        tx
    });
}

#[test]
fn it_can_apply_unstaking_transaction() {
    let key_pair = KeyPair::generate();
    let recipient = Address::from(&key_pair.public);
    let bls_pair = BlsKeyPair::generate(&mut thread_rng());
    let mut contract = make_sample_contract(&key_pair, &bls_pair);
    let fee = Coin::try_from(234).unwrap();

    let make_retire = |total_cost: u64| -> Transaction {
        let mut tx = make_outgoing_transaction();
        tx.recipient = tx.sender.clone();
        tx.value = Coin::try_from(total_cost).unwrap() - fee;
        tx.proof = SignatureProof::from(key_pair.public, key_pair.sign(&tx.serialize_content())).serialize_to_vec();
        tx
    };

    let make_unstake = |total_cost: u64| -> Transaction {
        let mut tx = make_outgoing_transaction();
        tx.recipient = recipient.clone();
        tx.value = Coin::try_from(total_cost).unwrap() - fee;
        tx.proof = SignatureProof::from(key_pair.public, key_pair.sign(&tx.serialize_content())).serialize_to_vec();
        tx
    };

    // Block 2: Retire first half of stake
    let tx_1 = make_retire(150_000_000 - 234);
    assert_eq!(contract.commit_outgoing_transaction(&tx_1, 2), Ok(None));
    assert_eq!(contract.commit_incoming_transaction(&tx_1, 2), Ok(None));
    assert_eq!(contract.balance, 299_999_766.try_into().unwrap());

    // Try to unstake too much
    let tx_2 = make_unstake(999_999_999);
    let funds_error = AccountError::InsufficientFunds {
        needed:  999_999_999.try_into().unwrap(),
        balance: 149_999_532.try_into().unwrap(),
    };
    assert_eq!(contract.check_outgoing_transaction(&tx_2, 40003), Err(funds_error.clone()));
    assert_eq!(contract.commit_outgoing_transaction(&tx_2, 40003), Err(funds_error.clone()));

    // Block 40003: Unstake quarter
    let tx_3 = make_unstake(75_000_000 - 234);
    assert_eq!(contract.check_outgoing_transaction(&tx_3, 40003), Ok(()));
    assert_eq!(contract.commit_outgoing_transaction(&tx_3, 40003), Ok(None));
    assert_eq!(contract.balance, 225_000_000.try_into().unwrap());

    // Block 40004: Unstake another quarter
    let tx_4 = tx_3.clone();
    assert_eq!(contract.check_outgoing_transaction(&tx_4, 40004), Ok(()));
    let receipt_4 = contract.commit_outgoing_transaction(&tx_4, 40004).unwrap().unwrap();
    assert_eq!(contract.balance, 150_000_234.try_into().unwrap());

    // Revert block 40004
    assert_eq!(contract.revert_outgoing_transaction(&tx_4, 40004, Some(&receipt_4)), Ok(()));

    // New block 40004: Retire second half of stake
    let tx_5 = make_retire(150_000_234);
    let receipt_5_outgoing = contract.commit_outgoing_transaction(&tx_5, 40004).unwrap().unwrap();
    let receipt_5_incoming = contract.commit_incoming_transaction(&tx_5, 40004).unwrap().unwrap();

    // Try to replay reverted unstaking, should fail
    let tx_6 = tx_3.clone();
    assert_eq!(contract.check_outgoing_transaction(&tx_6, 40005), Err(AccountError::InvalidForSender));
    assert_eq!(contract.commit_outgoing_transaction(&tx_6, 40005), Err(AccountError::InvalidForSender));

    // Unstake rest
    let tx_7 = make_unstake(225_000_000 - 234);
    assert_eq!(contract.check_outgoing_transaction(&tx_7, 100000), Ok(()));
    let receipt_7 = contract.commit_outgoing_transaction(&tx_7, 100000).unwrap().unwrap();
    assert_eq!(contract.balance, 0.try_into().unwrap());

    // Contract is empty at this point
    assert_eq!(contract.active_stake_by_address.len(), 0);
    assert_eq!(contract.active_stake_sorted.len(), 0);
    assert_eq!(contract.inactive_stake_by_address.len(), 0);
    assert_eq!(contract.balance, 0.try_into().unwrap());

    // Try to unstake nonexistent funds
    let tx_8 = tx_3.clone();
    assert_eq!(contract.check_outgoing_transaction(&tx_8, 40006), Err(AccountError::InvalidForSender));
    assert_eq!(contract.commit_outgoing_transaction(&tx_8, 40006), Err(AccountError::InvalidForSender));

    // Revert everything
    assert_eq!(contract.revert_outgoing_transaction(&tx_7, 100000, Some(&receipt_7)), Ok(()));
    assert_eq!(contract.revert_incoming_transaction(&tx_5, 40004, Some(&receipt_5_incoming)), Ok(()));
    assert_eq!(contract.revert_outgoing_transaction(&tx_5, 40004, Some(&receipt_5_outgoing)), Ok(()));
    assert_eq!(contract.revert_outgoing_transaction(&tx_3, 40003, None), Ok(()));
    assert_eq!(contract.revert_incoming_transaction(&tx_1, 2, None), Ok(()));
    assert_eq!(contract.revert_outgoing_transaction(&tx_1, 2, None), Ok(()));

    // Initial contract state
    assert_eq!(contract.active_stake_by_address.len(), 1);
    assert_eq!(contract.active_stake_sorted.len(), 1);
    assert_eq!(contract.inactive_stake_by_address.len(), 0);
    assert_eq!(contract.balance, 300_000_000.try_into().unwrap());
}



#[test]
fn it_can_build_a_validator_set() {
    // Helper function for building a staking transaction.
    // `order` sets the first byte of the address as a marker.
    // It also controls the secondary index when building the potential validator list.
    let stake = |amount: u64, order: u16| {
        let bls_pair = BlsKeyPair::generate(&mut thread_rng());
        let mut tx = make_incoming_transaction();
        tx.value = Coin::from_u64_unchecked(amount);
        let mut address_buf = [0u8; 20];
        address_buf[0] = (order & 0xFF) as u8;
        tx.sender = Address::from(address_buf);
        tx.data = StakingTransactionData {
            validator_key: bls_pair.public.clone(),
            reward_address: None,
            proof_of_knowledge: bls_pair.sign(&bls_pair.public),
        }.serialize_to_vec();
        tx
    };

    // Create arbitrary BLS signature as seed
    let bls_pair = BlsKeyPair::generate(&mut thread_rng());
    let seed = bls_pair.sign(&bls_pair.public);

    // Fill contract with same stakes
    let mut contract = make_empty_contract();
    contract.commit_incoming_transaction(&stake(10_000,  0xFE), 2).unwrap();
    contract.commit_incoming_transaction(&stake(130_000, 0x00), 2).unwrap();
    contract.commit_incoming_transaction(&stake(12,      0xFF), 2).unwrap();

    // Test potential validator selection by stake
    let validator_set = contract.build_validator_set(&seed, 1, 1);
    assert_eq!(validator_set.active.len(), 1);
    assert_eq!(validator_set.active[0].staking_address.as_bytes()[0], 0x00);

    // Fill contract with same stakes
    let mut contract = make_empty_contract();
    contract.commit_incoming_transaction(&stake(100_000_000, 0x03), 2).unwrap();
    contract.commit_incoming_transaction(&stake(100_000_000, 0xFF), 2).unwrap();
    contract.commit_incoming_transaction(&stake(100_000_000, 0x04), 2).unwrap();

    // Test potential validator selection by secondary index
    let validator_set = contract.build_validator_set(&seed, 1, 1);
    assert_eq!(validator_set.min_required_stake, Coin::from_u64_unchecked(100_000_000));
    assert_eq!(validator_set.active.len(), 1);
    assert_eq!(validator_set.active[0].staking_address.as_bytes()[0], 0x03);

    // TODO More tests
}

fn make_empty_contract() -> StakingContract {
    return StakingContract {
        balance: 0.try_into().unwrap(),
        active_stake_sorted: BTreeSet::new(),
        active_stake_by_address: HashMap::new(),
        inactive_stake_by_address: HashMap::new(),
    };
}

fn make_sample_contract(key_pair: &KeyPair, bls_pair: &BlsKeyPair) -> StakingContract {
    let mut contract = make_empty_contract();
    let mut tx = make_incoming_transaction();
    tx.value = 300_000_000.try_into().unwrap();
    tx.sender = Address::from(&key_pair.public);

    let proof_of_knowledge = bls_pair.sign_hash(Deserialize::deserialize_from_vec(&[0x41u8; 32].to_vec()).unwrap());

    let data = StakingTransactionData {
        validator_key: bls_pair.public.clone(),
        reward_address: Some(Address::from([3u8; 20])),
        proof_of_knowledge,
    };
    tx.data = data.serialize_to_vec();

    contract.commit_incoming_transaction(&tx, 2).expect("Failed to make sample contract");

    contract
}

fn make_incoming_transaction() -> Transaction {
    let mut tx = Transaction::new_basic(
        Address::from([2u8; 20]),
        Address::from([1u8; 20]),
        150_000_000.try_into().unwrap(),
        234.try_into().unwrap(),
        1, NetworkId::Dummy,
    );
    tx.recipient_type = AccountType::Staking;
    tx
}

fn make_outgoing_transaction() -> Transaction {
    let mut tx = Transaction::new_basic(
        Address::from([1u8; 20]),
        Address::from([2u8; 20]),
        149_999_766.try_into().unwrap(),
        234.try_into().unwrap(),
        1, NetworkId::Dummy,
    );
    tx.sender_type = AccountType::Staking;
    tx
}
