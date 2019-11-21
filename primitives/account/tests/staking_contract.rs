use std::convert::{TryFrom, TryInto};

use rand::thread_rng;

use beserial::{Deserialize, Serialize};
use nimiq_account::{AccountError, AccountTransactionInteraction, AccountType, StakingContract};
use nimiq_account::inherent::{AccountInherentInteraction, Inherent, InherentType};
use nimiq_bls::bls12_381::KeyPair as BlsKeyPair;
use nimiq_bls::bls12_381::Signature as BlsSignature;
use nimiq_bls::SecureGenerate;
use nimiq_keys::{Address, KeyPair, PrivateKey};
use nimiq_primitives::coin::Coin;
use nimiq_primitives::networks::NetworkId;
use nimiq_primitives::slot::{SlotCollection, SlotIndex};
use nimiq_transaction::{SignatureProof, Transaction, TransactionError};
use nimiq_transaction::account::AccountTransactionVerification;
use nimiq_transaction::account::staking_contract::{StakingTransactionData, StakingTransactionType};

const CONTRACT_1: &str = "000000000000000000000000000000000000000000000000";
const CONTRACT_2: &str = "0000000023c34600000000020202020202020202020202020202020202020202000000001ad27480a2f7d485efe6fabad3d780d1ea5ad690bd027a5328f44b612cad1f33347c8df5bde90a340c30877a21861e2173f6cfda0715d35ac2941437bf7e73d7e48fcf6e1901249134532ad1826ad1e396caed2d4d1d11e82d79f93946b21800a00971f000005e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e0000000008f0d180a9edd1613b714ec6107f4ffd532e52727c4f3a2897b3000e9ebccf076e8ffdf4b424f7e798d31dc67bbf9b3776096f101740b3f992ba8a5d0e20860f8d3466b7b58fb6b918eebb3c014bf6bb1cbdcb045c184d673c3db6435f454a1c530b9dfc012a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a00000000000000000000000000";

#[test]
fn it_can_de_serialize_a_staking_contract() {
    let bytes_1: Vec<u8> = hex::decode(CONTRACT_1).unwrap();
    let contract_1: StakingContract = Deserialize::deserialize(&mut &bytes_1[..]).unwrap();
    assert_eq!(contract_1.balance, 0.try_into().unwrap());
    assert_eq!(contract_1.active_stake_by_address.len(), 0);
    assert_eq!(contract_1.active_stake_sorted.len(), 0);
    assert_eq!(contract_1.inactive_stake_by_address.len(), 0);
    assert_eq!(contract_1.current_epoch_parking.len(), 0);
    assert_eq!(contract_1.previous_epoch_parking.len(), 0);
    let mut bytes_1_out = Vec::<u8>::with_capacity(contract_1.serialized_size());
    let size_1_out = contract_1.serialize(&mut bytes_1_out).unwrap();
    assert_eq!(size_1_out, contract_1.serialized_size());
    assert_eq!(hex::encode(bytes_1_out), CONTRACT_1);

    let bytes_2: Vec<u8> = hex::decode(CONTRACT_2).unwrap();
    let contract_2: StakingContract = Deserialize::deserialize(&mut &bytes_2[..]).unwrap();
    assert_eq!(contract_2.balance, 600_000_000.try_into().unwrap());
    assert_eq!(contract_2.get_balance(&Address::from([2u8; 20])), Coin::from_u64_unchecked(450_000_000u64));
    assert_eq!(contract_2.get_balance(&Address::from([0x5eu8; 20])), Coin::from_u64_unchecked(150_000_000u64));
    assert_eq!(contract_2.active_stake_by_address.len(), 2);
    assert_eq!(contract_2.active_stake_sorted.len(), 2);
    assert_eq!(contract_2.inactive_stake_by_address.len(), 0);
    assert_eq!(contract_2.current_epoch_parking.len(), 0);
    assert_eq!(contract_2.previous_epoch_parking.len(), 0);
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

    let proof_of_knowledge = bls_pair.sign(&bls_pair.public).compress();

    let mut data = StakingTransactionData {
        validator_key: bls_pair.public.compress(),
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
    data.proof_of_knowledge = invalid_pok.compress();
    tx.data = data.serialize_to_vec();
    assert_eq!(AccountType::verify_incoming_transaction(&tx), Err(TransactionError::InvalidData));
}

#[test]
fn it_can_apply_staking_transaction() {
    let mut contract = make_empty_contract();

    // Default transaction data
    let bls_pair = BlsKeyPair::generate(&mut thread_rng());
    let proof_of_knowledge = bls_pair.sign(&bls_pair.public).compress();
    let stake_data = StakingTransactionData {
        validator_key: bls_pair.public.compress(),
        reward_address: None,
        proof_of_knowledge,
    };

    // Default stake
    let mut tx_1 = make_incoming_transaction();
    tx_1.data = stake_data.serialize_to_vec();
    assert_eq!(StakingContract::check_incoming_transaction(&tx_1, 2), Ok(()));
    assert_eq!(contract.commit_incoming_transaction(&tx_1, 2), Ok(None));
    assert_eq!(contract.active_stake_by_address.len(), 1);
    assert_eq!(contract.get_balance(&Address::from([2u8; 20])), Coin::from_u64_unchecked(150_000_000u64));
    assert_eq!(contract.balance, 150_000_000.try_into().unwrap());

    // Same stake again
    let mut tx_2 = make_incoming_transaction();
    tx_2.data = stake_data.serialize_to_vec();
    assert_eq!(StakingContract::check_incoming_transaction(&tx_2, 3), Ok(()));
    let receipt_2 = contract.commit_incoming_transaction(&tx_2, 3).unwrap().unwrap();
    assert_eq!(contract.active_stake_by_address.len(), 1);
    assert_eq!(contract.get_balance(&Address::from([2u8; 20])), Coin::from_u64_unchecked(300_000_000u64));
    assert_eq!(contract.balance, 300_000_000.try_into().unwrap());

    // Stake again, changing validator key
    let mut tx_3 = make_incoming_transaction();
    let bls_other = BlsKeyPair::generate(&mut thread_rng());
    let pok_other = bls_other.sign(&bls_other.public);
    tx_3.data = StakingTransactionData {
        validator_key: bls_other.public.compress(),
        reward_address: None,
        proof_of_knowledge: pok_other.compress(),
    }.serialize_to_vec();
    assert_eq!(StakingContract::check_incoming_transaction(&tx_3, 4), Ok(()));
    let receipt_3 = contract.commit_incoming_transaction(&tx_3, 4).unwrap().unwrap();
    assert_eq!(contract.active_stake_by_address.len(), 1);
    assert_eq!(contract.get_balance(&Address::from([2u8; 20])), Coin::from_u64_unchecked(450_000_000u64));
    assert_eq!(contract.balance, 450_000_000.try_into().unwrap());

    // Stake on new account with reward address
    let mut tx_4 = make_incoming_transaction();
    tx_4.sender = Address::from([94u8; 20]);
    let mut stake_data_4 = stake_data.clone();
    stake_data_4.reward_address = Some(Address::from([42u8; 20]));
    tx_4.data = stake_data_4.serialize_to_vec();
    assert_eq!(StakingContract::check_incoming_transaction(&tx_4, 5), Ok(()));
    assert_eq!(contract.commit_incoming_transaction(&tx_4, 5), Ok(None));
    assert_eq!(contract.active_stake_by_address.len(), 2);
    assert_eq!(contract.get_balance(&Address::from([2u8; 20])), Coin::from_u64_unchecked(450_000_000u64));
    assert_eq!(contract.get_balance(&Address::from([94u8; 20])), Coin::from_u64_unchecked(150_000_000u64));
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
    let key_pair = KeyPair::generate(&mut thread_rng());
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
    let other_pair = KeyPair::generate(&mut thread_rng());
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
    let key_pair = KeyPair::generate(&mut thread_rng());
    let bls_pair = BlsKeyPair::generate(&mut thread_rng());
    let mut contract = make_sample_contract(&key_pair, &bls_pair);

    // Retire first half of stake
    let mut tx_1 = make_outgoing_transaction();
    tx_1.recipient = tx_1.sender.clone();
    tx_1.proof = SignatureProof::from(key_pair.public.clone(), key_pair.sign(&tx_1.serialize_content())).serialize_to_vec();
    tx_1.data = StakingTransactionType::Retire.serialize_to_vec();
    assert_eq!(contract.check_outgoing_transaction(&tx_1, 2), Ok(()));
    assert_eq!(contract.commit_outgoing_transaction(&tx_1, 2), Ok(None));
    assert_eq!(StakingContract::check_incoming_transaction(&tx_1, 2), Ok(()));
    assert_eq!(contract.commit_incoming_transaction(&tx_1, 2).unwrap(), None);

    assert_eq!(contract.active_stake_by_address.len(), 1);
    assert_eq!(contract.active_stake_sorted.len(), 1);
    assert_eq!(contract.inactive_stake_by_address.len(), 1);
    assert_eq!(contract.get_balance(&Address::from(&key_pair.public)), 299_999_766.try_into().unwrap());
    assert_eq!(contract.balance, 299_999_766.try_into().unwrap());

    // Try to retire too much stake
    let mut tx_2 = make_outgoing_transaction();
    tx_2.value = 200_000_000.try_into().unwrap();
    tx_2.recipient = tx_2.sender.clone();
    tx_2.proof = SignatureProof::from(key_pair.public, key_pair.sign(&tx_2.serialize_content())).serialize_to_vec();
    tx_2.data = StakingTransactionType::Retire.serialize_to_vec();
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
    tx_3.data = StakingTransactionType::Retire.serialize_to_vec();
    assert_eq!(contract.check_outgoing_transaction(&tx_3, 3), Ok(()));
    assert_eq!(contract.commit_outgoing_transaction(&tx_3, 3), Ok(None));
    assert_eq!(contract.check_outgoing_transaction(&tx_3, 3), Ok(()));
    let receipt_outgoing_2 = contract.commit_outgoing_transaction(&tx_3, 3).unwrap().unwrap();
    assert_eq!(StakingContract::check_incoming_transaction(&tx_3, 3), Ok(()));
    let receipt_incoming_1 = contract.commit_incoming_transaction(&tx_3, 3).unwrap().unwrap();
    assert_eq!(StakingContract::check_incoming_transaction(&tx_3, 3), Ok(()));
    let receipt_incoming_2 = contract.commit_incoming_transaction(&tx_3, 3).unwrap().unwrap();

    assert_eq!(contract.active_stake_by_address.len(), 0);
    assert_eq!(contract.active_stake_sorted.len(), 0);
    assert_eq!(contract.inactive_stake_by_address.len(), 1);
    assert_eq!(contract.get_balance(&Address::from(&key_pair.public)), 299_999_298.try_into().unwrap());
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
    assert_eq!(contract.get_balance(&Address::from(&key_pair.public)), 300_000_000.try_into().unwrap());
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
    let key_pair = KeyPair::generate(&mut thread_rng());
    let recipient = Address::from(&key_pair.public);
    let bls_pair = BlsKeyPair::generate(&mut thread_rng());
    let mut contract = make_sample_contract(&key_pair, &bls_pair);
    let fee = Coin::try_from(234).unwrap();

    let make_retire = |total_cost: u64| -> Transaction {
        let mut tx = make_outgoing_transaction();
        tx.recipient = tx.sender.clone();
        tx.value = Coin::try_from(total_cost).unwrap() - fee;
        tx.proof = SignatureProof::from(key_pair.public, key_pair.sign(&tx.serialize_content())).serialize_to_vec();
        tx.data = StakingTransactionType::Retire.serialize_to_vec();
        tx
    };

    let make_unstake = |total_cost: u64| -> Transaction {
        let mut tx = make_outgoing_transaction();
        tx.recipient = recipient.clone();
        tx.value = Coin::try_from(total_cost).unwrap() - fee;
        tx.proof = SignatureProof::from(key_pair.public, key_pair.sign(&tx.serialize_content())).serialize_to_vec();
        tx
    };

    let assert_balance = |contract: &StakingContract, value: u64| {
        assert_eq!(contract.get_balance(&Address::from(&key_pair.public)), value.try_into().unwrap());
        assert_eq!(contract.balance, Coin::from_u64_unchecked(value));
    };

    // Block 2: Retire first half of stake
    let tx_1 = make_retire(150_000_000 - 234);
    assert_eq!(contract.commit_outgoing_transaction(&tx_1, 2), Ok(None));
    assert_eq!(contract.commit_incoming_transaction(&tx_1, 2), Ok(None));
    assert_balance(&contract, 299_999_766);

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
    assert_balance(&contract, 225_000_000);

    // Block 40004: Unstake another quarter
    let tx_4 = tx_3.clone();
    assert_eq!(contract.check_outgoing_transaction(&tx_4, 40004), Ok(()));
    let receipt_4 = contract.commit_outgoing_transaction(&tx_4, 40004).unwrap().unwrap();
    assert_balance(&contract, 150_000_234);

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
    assert_balance(&contract, 300_000_000);
}

fn bls_key_pair() -> BlsKeyPair {
    const BLS_PRIVKEY: &str = "30a891c851e27600fefa7b0a84eac9caa645c98f2790e715fa09e49cb34fd73c";
    BlsKeyPair::from_secret(&Deserialize::deserialize(&mut &hex::decode(BLS_PRIVKEY).unwrap()[..]).unwrap())
}

fn ed25519_key_pair() -> KeyPair {
    const PRIVKEY: &str = "fc9b15259bf42d3e7415b75a41db8e3280280bffa7ffbe5903a5537ac9b45f75";
    let priv_key: PrivateKey = Deserialize::deserialize(&mut &hex::decode(PRIVKEY).unwrap()[..]).unwrap();
    priv_key.into()
}

#[test]
fn it_can_verify_inherent() {
    let key_pair = ed25519_key_pair();
    let mut contract = make_sample_contract(&key_pair, &bls_key_pair());

    // Reward inherent
    let inherent_1 = Inherent {
        ty: InherentType::Reward,
        target: Address::from([0u8; 20]),
        value: Coin::ZERO,
        data: Vec::new(),
    };
    assert_eq!(contract.check_inherent(&inherent_1, 0), Err(AccountError::InvalidForTarget));
    assert_eq!(contract.commit_inherent(&inherent_1, 0), Err(AccountError::InvalidForTarget));

    // Slash inherent to address without stake
    let mut inherent_2 = inherent_1.clone();
    inherent_2.ty = InherentType::Slash;
    inherent_2.data = Address::from([0u8; 20]).serialize_to_vec();
    assert_eq!(contract.check_inherent(&inherent_2, 0), Err(AccountError::InvalidInherent));
    assert_eq!(contract.commit_inherent(&inherent_2, 0), Err(AccountError::InvalidInherent));

    // Slash inherent with invalid data
    let mut inherent_3 = inherent_2.clone();
    inherent_3.value = Coin::from_u64_unchecked(38u64);
    inherent_3.data = Vec::from(&[42u8][..]);
    assert_eq!(contract.check_inherent(&inherent_3, 0), Err(AccountError::InvalidInherent));
    assert_eq!(contract.commit_inherent(&inherent_3, 0), Err(AccountError::InvalidInherent));
}

#[test]
fn it_rejects_invalid_slash_inherents() {
    let bls_pair = bls_key_pair();
    let key_pair = ed25519_key_pair();
    let mut contract = make_sample_contract(&key_pair, &bls_pair);
    let address = Address::from(&key_pair.public);

    // Invalid inherent
    let mut inherent = Inherent {
        ty: InherentType::Slash,
        target: Default::default(),
        value: Coin::from_u64_unchecked(1),
        data: address.serialize_to_vec(),
    };

    // Invalid value.
    assert_eq!(contract.check_inherent(&inherent, 0), Err(AccountError::InvalidInherent));
    assert_eq!(contract.commit_inherent(&inherent, 0), Err(AccountError::InvalidInherent));

    // Invalid data.
    inherent.value = Coin::ZERO;
    inherent.data = Vec::new();
    assert_eq!(contract.check_inherent(&inherent, 0), Err(AccountError::InvalidInherent));
    assert_eq!(contract.commit_inherent(&inherent, 0), Err(AccountError::InvalidInherent));
}

#[test]
fn it_rejects_invalid_finalize_epoch_inherents() {
    let bls_pair = bls_key_pair();
    let key_pair = ed25519_key_pair();
    let mut contract = make_sample_contract(&key_pair, &bls_pair);

    // Invalid inherent
    let mut inherent = Inherent {
        ty: InherentType::FinalizeEpoch,
        target: Default::default(),
        value: Coin::from_u64_unchecked(1),
        data: Vec::new(),
    };

    // Invalid value.
    assert_eq!(contract.check_inherent(&inherent, 0), Err(AccountError::InvalidInherent));
    assert_eq!(contract.commit_inherent(&inherent, 0), Err(AccountError::InvalidInherent));

    // Invalid data.
    inherent.value = Coin::ZERO;
    inherent.data = vec![1];
    assert_eq!(contract.check_inherent(&inherent, 0), Err(AccountError::InvalidInherent));
    assert_eq!(contract.commit_inherent(&inherent, 0), Err(AccountError::InvalidInherent));
}

#[test]
fn it_can_apply_slash_and_finalize_epoch_inherent() {
    let bls_pair = bls_key_pair();
    let key_pair = ed25519_key_pair();
    let mut contract = make_sample_contract(&key_pair, &bls_pair);
    let address = Address::from(&key_pair.public);

    let assert_balance = |contract: &StakingContract, value: u64| {
        assert_eq!(contract.get_balance(&Address::from(&key_pair.public)), value.try_into().unwrap());
        assert_eq!(contract.balance, Coin::from_u64_unchecked(value));
    };

    // Slash
    let slash = Inherent {
        ty: InherentType::Slash,
        target: Default::default(),
        value: Coin::ZERO,
        data: address.serialize_to_vec(),
    };
    assert_eq!(contract.check_inherent(&slash, 0), Ok(()));
    assert_eq!(contract.commit_inherent(&slash, 0), Ok(Some(vec![1]))); // Receipt is boolean set to true.
    assert_balance(&contract, 300_000_000);
    assert_eq!(contract.get_active_balance(&address), Coin::from_u64_unchecked(300_000_000));
    assert_eq!(contract.current_epoch_parking.len(), 1);
    assert_eq!(contract.previous_epoch_parking.len(), 0);
    assert!(contract.current_epoch_parking.contains(&address));

    // Second slash
    assert_eq!(contract.check_inherent(&slash, 0), Ok(()));
    assert_eq!(contract.commit_inherent(&slash, 0), Ok(Some(vec![0])));  // Receipt is boolean set to false.
    assert_balance(&contract, 300_000_000);
    assert_eq!(contract.get_active_balance(&address), Coin::from_u64_unchecked(300_000_000));
    assert_eq!(contract.current_epoch_parking.len(), 1);
    assert_eq!(contract.previous_epoch_parking.len(), 0);
    assert!(contract.current_epoch_parking.contains(&address));

    // First finalize
    let finalize = Inherent {
        ty: InherentType::FinalizeEpoch,
        target: Default::default(),
        value: Coin::ZERO,
        data: vec![]
    };

    assert_eq!(contract.check_inherent(&finalize, 0), Ok(()));
    assert_eq!(contract.commit_inherent(&finalize, 0), Ok(None));
    assert_balance(&contract, 300_000_000);
    assert_eq!(contract.get_active_balance(&address), Coin::from_u64_unchecked(300_000_000));
    assert_eq!(contract.current_epoch_parking.len(), 0);
    assert_eq!(contract.previous_epoch_parking.len(), 1);
    assert!(contract.previous_epoch_parking.contains(&address));

    // Third slash
    assert_eq!(contract.check_inherent(&slash, 0), Ok(()));
    assert_eq!(contract.commit_inherent(&slash, 0), Ok(Some(vec![1])));
    assert_balance(&contract, 300_000_000);
    assert_eq!(contract.get_active_balance(&address), Coin::from_u64_unchecked(300_000_000));
    assert_eq!(contract.current_epoch_parking.len(), 1);
    assert_eq!(contract.previous_epoch_parking.len(), 1);
    assert!(contract.current_epoch_parking.contains(&address));
    assert!(contract.previous_epoch_parking.contains(&address));

    // Another finalize
    assert_eq!(contract.check_inherent(&finalize, 0), Ok(()));
    assert_eq!(contract.commit_inherent(&finalize, 0), Ok(None));
    assert_balance(&contract, 300_000_000);
    assert_eq!(contract.get_inactive_balance(&address), Coin::from_u64_unchecked(300_000_000));
    assert_eq!(contract.current_epoch_parking.len(), 0);
    assert_eq!(contract.previous_epoch_parking.len(), 1);
    assert!(contract.previous_epoch_parking.contains(&address));

    // Another finalize
    assert_eq!(contract.check_inherent(&finalize, 0), Ok(()));
    assert_eq!(contract.commit_inherent(&finalize, 0), Ok(None));
    assert_balance(&contract, 300_000_000);
    assert_eq!(contract.get_inactive_balance(&address), Coin::from_u64_unchecked(300_000_000));
    assert_eq!(contract.current_epoch_parking.len(), 0);
    assert_eq!(contract.previous_epoch_parking.len(), 0);
}

#[test]
fn it_can_apply_slashes_after_retire() {
    // In this test, we are retiring a staker before it gets slashed and test two scenarios.
    // retire
    //   |
    // slash
    //   |   \
    //   |     \
    // finalize revert slash
    //   |
    // finalize
    let key_pair = KeyPair::generate(&mut thread_rng());
    let address = Address::from(&key_pair.public);
    let bls_pair = BlsKeyPair::generate(&mut thread_rng());
    let mut contract = make_sample_contract(&key_pair, &bls_pair);

    // Retire full stake
    let mut tx_1 = make_outgoing_transaction();
    tx_1.value = Coin::from_u64_unchecked(299_999_766);
    tx_1.recipient = tx_1.sender.clone();
    tx_1.proof = SignatureProof::from(key_pair.public.clone(), key_pair.sign(&tx_1.serialize_content())).serialize_to_vec();
    tx_1.data = StakingTransactionType::Retire.serialize_to_vec();
    assert_eq!(contract.check_outgoing_transaction(&tx_1, 2), Ok(()));
    assert!(contract.commit_outgoing_transaction(&tx_1, 2).is_ok());
    assert_eq!(StakingContract::check_incoming_transaction(&tx_1, 2), Ok(()));
    assert_eq!(contract.commit_incoming_transaction(&tx_1, 2).unwrap(), None);

    assert_eq!(contract.active_stake_by_address.len(), 0);
    assert_eq!(contract.active_stake_sorted.len(), 0);
    assert_eq!(contract.inactive_stake_by_address.len(), 1);
    assert_eq!(contract.get_balance(&Address::from(&key_pair.public)), 299_999_766.try_into().unwrap());
    assert_eq!(contract.balance, 299_999_766.try_into().unwrap());

    // Slash
    let slash = Inherent {
        ty: InherentType::Slash,
        target: Default::default(),
        value: Coin::ZERO,
        data: address.serialize_to_vec(),
    };
    assert_eq!(contract.check_inherent(&slash, 3), Ok(()));
    assert_eq!(contract.commit_inherent(&slash, 3), Ok(Some(vec![1]))); // Receipt is boolean set to true.
    assert_eq!(contract.get_active_balance(&address), Coin::from_u64_unchecked(0));
    assert_eq!(contract.get_inactive_balance(&address), Coin::from_u64_unchecked(299_999_766));
    assert_eq!(contract.current_epoch_parking.len(), 1);
    assert_eq!(contract.previous_epoch_parking.len(), 0);
    assert!(contract.current_epoch_parking.contains(&address));

    // Scenario 1: Revert slash
    let mut contract_copy = contract.clone();
    assert_eq!(contract_copy.revert_inherent(&slash, 3, Some(&vec![1])), Ok(()));
    assert_eq!(contract.get_active_balance(&address), Coin::from_u64_unchecked(0));
    assert_eq!(contract.get_inactive_balance(&address), Coin::from_u64_unchecked(299_999_766));
    assert_eq!(contract_copy.current_epoch_parking.len(), 0);
    assert_eq!(contract_copy.previous_epoch_parking.len(), 0);

    // Scenario 2: First finalize
    let finalize = Inherent {
        ty: InherentType::FinalizeEpoch,
        target: Default::default(),
        value: Coin::ZERO,
        data: vec![]
    };
    assert_eq!(contract.check_inherent(&finalize, 0), Ok(()));
    assert_eq!(contract.commit_inherent(&finalize, 0), Ok(None));
    assert_eq!(contract.get_active_balance(&address), Coin::from_u64_unchecked(0));
    assert_eq!(contract.get_inactive_balance(&address), Coin::from_u64_unchecked(299_999_766));
    assert_eq!(contract.current_epoch_parking.len(), 0);
    assert_eq!(contract.previous_epoch_parking.len(), 1);
    assert!(contract.previous_epoch_parking.contains(&address));

    // Second finalize
    assert_eq!(contract.check_inherent(&finalize, 0), Ok(()));
    assert_eq!(contract.commit_inherent(&finalize, 0), Ok(None));
    assert_eq!(contract.get_active_balance(&address), Coin::from_u64_unchecked(0));
    assert_eq!(contract.get_inactive_balance(&address), Coin::from_u64_unchecked(299_999_766));
    assert_eq!(contract.current_epoch_parking.len(), 0);
    assert_eq!(contract.previous_epoch_parking.len(), 0);
}

#[test]
fn it_can_apply_unpark_transactions() {
    let bls_pair = bls_key_pair();
    let key_pair = ed25519_key_pair();
    let mut contract = make_sample_contract(&key_pair, &bls_pair);
    let address = Address::from(&key_pair.public);

    let assert_balance = |contract: &StakingContract, value: u64| {
        assert_eq!(contract.get_balance(&Address::from(&key_pair.public)), value.try_into().unwrap());
        assert_eq!(contract.balance, Coin::from_u64_unchecked(value));
    };

    let make_unpark = |total_cost: u64, fee: u64| -> Transaction {
        let mut tx = make_outgoing_transaction();
        tx.recipient = tx.sender.clone();
        tx.value = Coin::try_from(total_cost - fee).unwrap();
        tx.fee = Coin::try_from(fee).unwrap();
        tx.proof = SignatureProof::from(key_pair.public, key_pair.sign(&tx.serialize_content())).serialize_to_vec();
        tx.data = StakingTransactionType::Unpark.serialize_to_vec();
        tx
    };

    // Unpark with invalid value
    let unpark = make_unpark(2, 2);
    assert_eq!(contract.check_outgoing_transaction(&unpark, 2), Err(AccountError::InvalidForSender));

    // Invalid type
    let mut unpark = make_unpark(300_000_000, 2);
    unpark.data = Vec::new();
    if let Err(AccountError::InvalidSerialization(_e)) = contract.check_outgoing_transaction(&unpark, 2) {
        // Ok
    } else {
        assert!(false, "Transaction should have been rejected.");
    }

    // Unpark with address that is not staked
    let priv_key: PrivateKey = Deserialize::deserialize(&mut &hex::decode("aa9b15259bf42d3e74efc25a41db8e3223280bffa7ffbe5903a5537ac9b43275").unwrap()[..]).unwrap();
    let key_pair2: KeyPair = priv_key.into();
    let mut unpark = make_unpark(300_000_000, 2);
    assert_ne!(Address::from(&key_pair2.public), address);
    unpark.proof = SignatureProof::from(key_pair2.public, key_pair2.sign(&unpark.serialize_content())).serialize_to_vec();
    assert_eq!(contract.check_outgoing_transaction(&unpark, 2), Err(AccountError::InvalidForSender));

    // Unpark with address that is not parked
    let unpark = make_unpark(300_000_000, 2);
    assert_eq!(contract.check_outgoing_transaction(&unpark, 2), Err(AccountError::InvalidForSender));

    // Slash
    let slash = Inherent {
        ty: InherentType::Slash,
        target: Default::default(),
        value: Coin::ZERO,
        data: address.serialize_to_vec(),
    };
    assert_eq!(contract.check_inherent(&slash, 0), Ok(()));
    assert_eq!(contract.commit_inherent(&slash, 0), Ok(Some(vec![1]))); // Receipt is boolean set to true.

    // Unpark
    let mut contract_copy = contract.clone();
    assert_eq!(contract_copy.check_outgoing_transaction(&unpark, 2), Ok(()));
    assert_eq!(StakingContract::check_incoming_transaction(&unpark, 2), Ok(()));
    assert!(contract_copy.commit_outgoing_transaction(&unpark, 2).is_ok());
    assert!(contract_copy.commit_incoming_transaction(&unpark, 2).is_ok());
    assert_balance(&contract_copy, 299_999_998);
    assert_eq!(contract_copy.get_active_balance(&address), Coin::from_u64_unchecked(299_999_998));
    assert_eq!(contract_copy.current_epoch_parking.len(), 0);
    assert_eq!(contract_copy.previous_epoch_parking.len(), 0);

    // Build on previous contract state and finalize and slash
    let finalize = Inherent {
        ty: InherentType::FinalizeEpoch,
        target: Default::default(),
        value: Coin::ZERO,
        data: vec![]
    };
    assert_eq!(contract.check_inherent(&finalize, 0), Ok(()));
    assert_eq!(contract.commit_inherent(&finalize, 0), Ok(None));
    assert_eq!(contract.check_inherent(&slash, 0), Ok(()));
    assert_eq!(contract.commit_inherent(&slash, 0), Ok(Some(vec![1])));
    assert_eq!(contract.current_epoch_parking.len(), 1);
    assert_eq!(contract.previous_epoch_parking.len(), 1);
    assert_balance(&contract, 300_000_000);

    // Unpark
    let unpark = make_unpark(300_000_000, 2);
    assert_eq!(contract.check_outgoing_transaction(&unpark, 2), Ok(()));
    assert_eq!(StakingContract::check_incoming_transaction(&unpark, 2), Ok(()));
    assert!(contract.commit_outgoing_transaction(&unpark, 2).is_ok());
    assert!(contract.commit_incoming_transaction(&unpark, 2).is_ok());
    assert_balance(&contract, 299_999_998);
    assert_eq!(contract.get_active_balance(&address), Coin::from_u64_unchecked(299_999_998));
    assert_eq!(contract.current_epoch_parking.len(), 0);
    assert_eq!(contract.previous_epoch_parking.len(), 0);
}

#[test]
fn it_can_revert_unpark_transactions() {
    let bls_pair = bls_key_pair();
    let key_pair = ed25519_key_pair();
    let contract = make_sample_contract(&key_pair, &bls_pair);
    let address = Address::from(&key_pair.public);

    let assert_balance = |contract: &StakingContract, value: u64| {
        assert_eq!(contract.get_balance(&Address::from(&key_pair.public)), value.try_into().unwrap());
        assert_eq!(contract.balance, Coin::from_u64_unchecked(value));
    };

    let mut unpark = make_outgoing_transaction();
    unpark.recipient = unpark.sender.clone();
    unpark.value = Coin::try_from(299_999_998).unwrap();
    unpark.fee = Coin::try_from(2).unwrap();
    unpark.proof = SignatureProof::from(key_pair.public, key_pair.sign(&unpark.serialize_content())).serialize_to_vec();
    unpark.data = StakingTransactionType::Unpark.serialize_to_vec();

    // Slash
    let mut parked_in_current = contract;
    let slash = Inherent {
        ty: InherentType::Slash,
        target: Default::default(),
        value: Coin::ZERO,
        data: address.serialize_to_vec(),
    };
    assert_eq!(parked_in_current.check_inherent(&slash, 0), Ok(()));
    assert_eq!(parked_in_current.commit_inherent(&slash, 0), Ok(Some(vec![1]))); // Receipt is boolean set to true.

    // Unpark
    let outgoing_receipt = parked_in_current.commit_outgoing_transaction(&unpark, 2);
    let incoming_receipt = parked_in_current.commit_incoming_transaction(&unpark, 2);
    assert!(incoming_receipt.is_ok());
    assert!(outgoing_receipt.is_ok());
    assert_balance(&parked_in_current, 299_999_998);
    assert_eq!(parked_in_current.get_active_balance(&address), Coin::from_u64_unchecked(299_999_998));
    assert_eq!(parked_in_current.current_epoch_parking.len(), 0);
    assert_eq!(parked_in_current.previous_epoch_parking.len(), 0);

    // Revert unpark
    assert_eq!(parked_in_current.revert_incoming_transaction(&unpark, 2, incoming_receipt.unwrap().as_ref()), Ok(()));
    assert_eq!(parked_in_current.revert_outgoing_transaction(&unpark, 2, outgoing_receipt.unwrap().as_ref()), Ok(()));
    assert_balance(&parked_in_current, 300_000_000);
    assert_eq!(parked_in_current.get_active_balance(&address), Coin::from_u64_unchecked(300_000_000));
    assert_eq!(parked_in_current.current_epoch_parking.len(), 1);
    assert!(parked_in_current.current_epoch_parking.contains(&address));
    assert_eq!(parked_in_current.previous_epoch_parking.len(), 0);

    // Park, unpark and revert unpark in previous epoch
    let mut parked_in_previous = parked_in_current.clone();
    let finalize = Inherent {
        ty: InherentType::FinalizeEpoch,
        target: Default::default(),
        value: Coin::ZERO,
        data: vec![]
    };
    assert_eq!(parked_in_previous.check_inherent(&finalize, 0), Ok(()));
    assert_eq!(parked_in_previous.commit_inherent(&finalize, 0), Ok(None));
    assert_eq!(parked_in_previous.current_epoch_parking.len(), 0);
    assert_eq!(parked_in_previous.previous_epoch_parking.len(), 1);
    assert!(parked_in_previous.previous_epoch_parking.contains(&address));

    // Unpark
    let outgoing_receipt = parked_in_previous.commit_outgoing_transaction(&unpark, 2);
    let incoming_receipt = parked_in_previous.commit_incoming_transaction(&unpark, 2);
    assert!(incoming_receipt.is_ok());
    assert!(outgoing_receipt.is_ok());
    assert_balance(&parked_in_previous, 299_999_998);
    assert_eq!(parked_in_previous.get_active_balance(&address), Coin::from_u64_unchecked(299_999_998));
    assert_eq!(parked_in_previous.current_epoch_parking.len(), 0);
    assert_eq!(parked_in_previous.previous_epoch_parking.len(), 0);

    // Revert unpark
    assert_eq!(parked_in_previous.revert_incoming_transaction(&unpark, 2, incoming_receipt.unwrap().as_ref()), Ok(()));
    assert_eq!(parked_in_previous.revert_outgoing_transaction(&unpark, 2, outgoing_receipt.unwrap().as_ref()), Ok(()));
    assert_balance(&parked_in_previous, 300_000_000);
    assert_eq!(parked_in_previous.get_active_balance(&address), Coin::from_u64_unchecked(300_000_000));
    assert_eq!(parked_in_previous.current_epoch_parking.len(), 0);
    assert_eq!(parked_in_previous.previous_epoch_parking.len(), 1);
    assert!(parked_in_previous.previous_epoch_parking.contains(&address));

    // Park, unpark and revert unpark in both epochs
    let mut parked_in_both = parked_in_current;
    let finalize = Inherent {
        ty: InherentType::FinalizeEpoch,
        target: Default::default(),
        value: Coin::ZERO,
        data: vec![]
    };
    assert_eq!(parked_in_both.check_inherent(&finalize, 0), Ok(()));
    assert_eq!(parked_in_both.commit_inherent(&finalize, 0), Ok(None));
    assert_eq!(parked_in_both.check_inherent(&slash, 0), Ok(()));
    assert_eq!(parked_in_both.commit_inherent(&slash, 0), Ok(Some(vec![1])));
    assert_eq!(parked_in_both.current_epoch_parking.len(), 1);
    assert!(parked_in_both.current_epoch_parking.contains(&address));
    assert_eq!(parked_in_both.previous_epoch_parking.len(), 1);
    assert!(parked_in_both.previous_epoch_parking.contains(&address));

    // Unpark
    let outgoing_receipt = parked_in_both.commit_outgoing_transaction(&unpark, 2);
    let incoming_receipt = parked_in_both.commit_incoming_transaction(&unpark, 2);
    assert!(incoming_receipt.is_ok());
    assert!(outgoing_receipt.is_ok());
    assert_balance(&parked_in_both, 299_999_998);
    assert_eq!(parked_in_both.get_active_balance(&address), Coin::from_u64_unchecked(299_999_998));
    assert_eq!(parked_in_both.current_epoch_parking.len(), 0);
    assert_eq!(parked_in_both.previous_epoch_parking.len(), 0);

    // Revert unpark
    assert_eq!(parked_in_both.revert_incoming_transaction(&unpark, 2, incoming_receipt.unwrap().as_ref()), Ok(()));
    assert_eq!(parked_in_both.revert_outgoing_transaction(&unpark, 2, outgoing_receipt.unwrap().as_ref()), Ok(()));
    assert_balance(&parked_in_both, 300_000_000);
    assert_eq!(parked_in_both.get_active_balance(&address), Coin::from_u64_unchecked(300_000_000));
    assert_eq!(parked_in_both.current_epoch_parking.len(), 1);
    assert!(parked_in_both.current_epoch_parking.contains(&address));
    assert_eq!(parked_in_both.previous_epoch_parking.len(), 1);
    assert!(parked_in_both.previous_epoch_parking.contains(&address));
}

#[test]
fn it_will_never_revert_finalized_epoch_inherents() {
    let bls_pair = bls_key_pair();
    let key_pair = ed25519_key_pair();
    let mut contract = make_sample_contract(&key_pair, &bls_pair);

    let finalize = Inherent {
        ty: InherentType::FinalizeEpoch,
        target: Default::default(),
        value: Coin::ZERO,
        data: vec![]
    };

    assert_eq!(contract.check_inherent(&finalize, 0), Ok(()));
    assert_eq!(contract.commit_inherent(&finalize, 0), Ok(None));
    assert_eq!(contract.revert_inherent(&finalize, 0, None), Err(AccountError::InvalidForTarget));
}

#[test]
fn it_can_revert_slash_inherent() {
    let bls_pair = bls_key_pair();
    let key_pair = ed25519_key_pair();
    let mut contract = make_sample_contract(&key_pair, &bls_pair);
    let address = Address::from(&key_pair.public);

    let assert_balance = |contract: &StakingContract, value: u64| {
        assert_eq!(contract.get_balance(&Address::from(&key_pair.public)), value.try_into().unwrap());
        assert_eq!(contract.balance, Coin::from_u64_unchecked(value));
    };

    // Slash
    let slash = Inherent {
        ty: InherentType::Slash,
        target: Default::default(),
        value: Coin::ZERO,
        data: address.serialize_to_vec(),
    };
    assert_eq!(contract.check_inherent(&slash, 0), Ok(()));
    assert_eq!(contract.commit_inherent(&slash, 0), Ok(Some(vec![1]))); // Receipt is boolean set to true.
    assert_balance(&contract, 300_000_000);
    assert_eq!(contract.get_active_balance(&address), Coin::from_u64_unchecked(300_000_000));
    assert_eq!(contract.current_epoch_parking.len(), 1);
    assert_eq!(contract.previous_epoch_parking.len(), 0);
    assert!(contract.current_epoch_parking.contains(&address));

    // Revert slash
    let mut contract_copy = contract.clone();
    assert_eq!(contract_copy.revert_inherent(&slash, 0, Some(&vec![1])), Ok(()));
    assert_balance(&contract_copy, 300_000_000);
    assert_eq!(contract_copy.get_active_balance(&address), Coin::from_u64_unchecked(300_000_000));
    assert_eq!(contract_copy.current_epoch_parking.len(), 0);
    assert_eq!(contract_copy.previous_epoch_parking.len(), 0);

    // First finalize
    let finalize = Inherent {
        ty: InherentType::FinalizeEpoch,
        target: Default::default(),
        value: Coin::ZERO,
        data: vec![]
    };
    assert_eq!(contract.check_inherent(&finalize, 0), Ok(()));
    assert_eq!(contract.commit_inherent(&finalize, 0), Ok(None));
    assert_balance(&contract, 300_000_000);
    assert_eq!(contract.get_active_balance(&address), Coin::from_u64_unchecked(300_000_000));
    assert_eq!(contract.current_epoch_parking.len(), 0);
    assert_eq!(contract.previous_epoch_parking.len(), 1);
    assert!(contract.previous_epoch_parking.contains(&address));

    // Revert slash after finalize is impossible
    let mut contract_copy = contract.clone();
    assert_eq!(contract_copy.revert_inherent(&slash, 0, Some(&vec![1])), Err(AccountError::InvalidInherent));

    // Slash multiple times and revert one of the slashes
    // This should *not* remove the slash
    // Slash 1
    assert_eq!(contract.check_inherent(&slash, 0), Ok(()));
    assert_eq!(contract.commit_inherent(&slash, 0), Ok(Some(vec![1]))); // Receipt is boolean set to true.
    assert_balance(&contract, 300_000_000);
    assert_eq!(contract.get_active_balance(&address), Coin::from_u64_unchecked(300_000_000));
    assert_eq!(contract.current_epoch_parking.len(), 1);
    assert_eq!(contract.previous_epoch_parking.len(), 1);
    assert!(contract.current_epoch_parking.contains(&address));
    assert!(contract.previous_epoch_parking.contains(&address));

    // Slash 2
    assert_eq!(contract.check_inherent(&slash, 0), Ok(()));
    assert_eq!(contract.commit_inherent(&slash, 0), Ok(Some(vec![0]))); // Receipt is boolean set to false.
    assert_balance(&contract, 300_000_000);
    assert_eq!(contract.get_active_balance(&address), Coin::from_u64_unchecked(300_000_000));
    assert_eq!(contract.current_epoch_parking.len(), 1);
    assert_eq!(contract.previous_epoch_parking.len(), 1);
    assert!(contract.current_epoch_parking.contains(&address));
    assert!(contract.previous_epoch_parking.contains(&address));

    // Revert second slash
    assert_eq!(contract.revert_inherent(&slash, 0, Some(&vec![0])), Ok(()));
    assert_balance(&contract, 300_000_000);
    assert_eq!(contract.get_active_balance(&address), Coin::from_u64_unchecked(300_000_000));
    assert_eq!(contract.current_epoch_parking.len(), 1);
    assert_eq!(contract.previous_epoch_parking.len(), 1);
    assert!(contract.current_epoch_parking.contains(&address));
    assert!(contract.previous_epoch_parking.contains(&address));

    // Revert first slash
    assert_eq!(contract.revert_inherent(&slash, 0, Some(&vec![1])), Ok(()));
    assert_balance(&contract, 300_000_000);
    assert_eq!(contract.get_active_balance(&address), Coin::from_u64_unchecked(300_000_000));
    assert_eq!(contract.current_epoch_parking.len(), 0);
    assert_eq!(contract.previous_epoch_parking.len(), 1);
    assert!(contract.previous_epoch_parking.contains(&address));
}

#[test]
fn it_can_build_a_validator_set() {
    // Helper function for building a staking transaction.
    // `order` sets the first byte of the address as a marker.
    // It also controls the secondary index when building the potential validator list.
    let stake = |amount: u64, order: u16| {
        let bls_pair = hex::decode("2cdd86b59112cf418f7c0dbaa831787f215a5fed7cac623290e48b4d744d038b").unwrap();
        let bls_pair = BlsKeyPair::deserialize_from_vec(&bls_pair).unwrap();
        let mut tx = make_incoming_transaction();
        tx.value = Coin::from_u64_unchecked(amount);
        let mut address_buf = [0u8; 20];
        address_buf[0] = (order & 0xFF) as u8;
        tx.sender = Address::from(address_buf);
        tx.data = StakingTransactionData {
            validator_key: bls_pair.public.compress(),
            reward_address: None,
            proof_of_knowledge: bls_pair.sign(&bls_pair.public).compress(),
        }.serialize_to_vec();
        tx
    };

    // Create arbitrary BLS signature as seed
    let seed_vec = hex::decode("ac22bbbf6a315f9e9eb23eca98918a0a5a35e31219b8c3c8b3bd5b71bc7a33371aad8588007e89e95ffe63bd9dce4c27").unwrap();
    let seed = BlsSignature::deserialize_from_vec(&seed_vec).unwrap();

    // Fill contract with same stakes
    let mut contract = make_empty_contract();
    contract.commit_incoming_transaction(&stake(10_000,  0xFE), 2).unwrap();
    contract.commit_incoming_transaction(&stake(130_000, 0x00), 2).unwrap();
    contract.commit_incoming_transaction(&stake(12,      0xFF), 2).unwrap();

    // Test potential validator selection by stake
    let slots = contract.select_validators(&seed.compress().into());
    assert_eq!(slots.stake_slots.len(), 2);
    assert_eq!(slots.get(SlotIndex::Slot(0)).unwrap().staker_address().as_bytes()[0], 0x00);

    // Fill contract with same stakes
    let mut contract = make_empty_contract();
    contract.commit_incoming_transaction(&stake(100_000_000, 0x03), 2).unwrap();
    contract.commit_incoming_transaction(&stake(100_000_000, 0xFF), 2).unwrap();
    contract.commit_incoming_transaction(&stake(100_000_000, 0x04), 2).unwrap();

    // Test potential validator selection by secondary index
    let slots = contract.select_validators(&seed.compress().into());
    assert_eq!(slots.stake_slots.len(), 3);
    assert_eq!(slots.get(SlotIndex::Slot(0)).unwrap().staker_address().as_bytes()[0], 0x03);

    // TODO More tests
}

fn make_empty_contract() -> StakingContract {
    StakingContract::default()
}

fn make_sample_contract(key_pair: &KeyPair, bls_pair: &BlsKeyPair) -> StakingContract {
    let mut contract = make_empty_contract();
    let mut tx = make_incoming_transaction();
    tx.value = 300_000_000.try_into().unwrap();
    tx.sender = Address::from(&key_pair.public);

    let proof_of_knowledge = bls_pair.sign_hash(Deserialize::deserialize_from_vec(&[0x41u8; 32].to_vec()).unwrap());

    let data = StakingTransactionData {
        validator_key: bls_pair.public.compress(),
        reward_address: Some(Address::from([3u8; 20])),
        proof_of_knowledge: proof_of_knowledge.compress(),
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
