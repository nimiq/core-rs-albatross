use std::collections::{BTreeMap, BTreeSet};
use std::convert::TryInto;

use beserial::{Deserialize, Serialize};
use nimiq_account::*;
use nimiq_bls::KeyPair as BlsKeyPair;
use nimiq_bls::SecretKey as BlsSecretKey;
use nimiq_bls::{CompressedPublicKey as BlsPublicKey, CompressedPublicKey};
use nimiq_collections::BitSet;
use nimiq_database::volatile::VolatileEnvironment;
use nimiq_database::WriteTransaction;
use nimiq_hash::Blake2bHash;
use nimiq_keys::{Address, KeyPair, PrivateKey, PublicKey};
use nimiq_primitives::account::AccountType;
use nimiq_primitives::coin::Coin;
use nimiq_primitives::networks::NetworkId;
use nimiq_primitives::policy;
use nimiq_primitives::policy::{
    BLOCKS_PER_BATCH, BLOCKS_PER_EPOCH, STAKING_CONTRACT_ADDRESS, VALIDATOR_DEPOSIT,
};
use nimiq_primitives::slots::SlashedSlot;
use nimiq_transaction::account::staking_contract::{
    IncomingStakingTransactionData, OutgoingStakingTransactionProof,
};
use nimiq_transaction::{SignatureProof, Transaction};
use nimiq_utils::key_rng::SecureGenerate;

const CONTRACT_1: &str = "00000000000000000000000000000000000000000000";
const CONTRACT_2: &str =
    "0000000011e1a3000000000100000000000000000000000000000000000000000000000011e1a30000000001010101010101010101010101010101010101010101000000000000040102000000000000000000000170000000000001010101010101010101010101010101010101010100020000000a0001020202020202020202020202020202020202020200040064006500660068";

const VALIDATOR_ADDRESS: &str = "83fa05dbe31f85e719f4c4fd67ebdba2e444d9f8";
const VALIDATOR_PRIVATE_KEY: &str =
    "d0fbb3690f5308f457e245a3cc65ae8d6945155eadcac60d489ffc5583a60b9b";

const VALIDATOR_SIGNING_KEY: &str =
    "b300481ddd7af6be3cf5c123b7af2c21f87f4ac808c8b0e622eb85826124a844";
const VALIDATOR_SIGNING_SECRET_KEY: &str =
    "84c961b11b52a8244ffc5e9d0965bc2dfa6764970f8e4989d45901de401baf27";

const VALIDATOR_VOTING_KEY: &str = "003d4e4eb0fa2fee42501368dc41115f64741e9d9496bbc2fe4cfd407f10272eef87b839d6e25b0eb7338427d895e4209190b6c5aa580f134693623a30ebafdaf95a268b3b84a840fc45d06283d71fe4faa2c7d08cd431bbda165c53a50453015a49ca120626991ff9558be65a7958158387829d6e56e2861e80b85e8c795d93f907afb19e6e2e5aaed9a3158eac5a035189986ff5803dd18fa02bdf5535e5495ed96990665ec165b3ba86fc1a7f7dabeb0510e1823813bf5ab1a01b4fff00bcd0373bc265efa135f8755ebae72b645a890d27ce8af31417347bc3a1d9cf09db339b68d1c9a50bb9c00faeedbefe9bab5a63b580e5f79c4a30dc1bdacccec0fc6a08e0853518e88557001a612d4c30d2fbc2a126a066a94f299ac5ce61";
const VALIDATOR_VOTING_SECRET_KEY: &str =
    "b552baff2c2cc4937ec3531c833c3ffc08f92a95b3ba4a53cf7e8c99ef9db99b99559b8dbb8f3c44fa5671da42cc2633759aea71c1b696ea18df5451d0d43a225a882b29a1091ece16e82f664c2c6f2b360c7b6ce84e5d0995ae45290dbd0000";

const STAKER_ADDRESS: &str = "8c551fabc6e6e00c609c3f0313257ad7e835643c";
const STAKER_PRIVATE_KEY: &str = "62f21a296f00562c43999094587d02c0001676ddbd3f0acf9318efbcad0c8b43";

// The following code is kept as a reference on how to generate the constants.
#[ignore]
#[test]
fn generate_contract_2() {
    let mut active_validators = BTreeMap::new();
    active_validators.insert(
        Address::from([0u8; 20]),
        Coin::from_u64_unchecked(300_000_000),
    );

    let mut parked_set = BTreeSet::new();
    parked_set.insert(Address::from([1u8; 20]));

    let mut current_lost_rewards = BitSet::new();
    current_lost_rewards.insert(0);
    current_lost_rewards.insert(10);

    let mut previous_lost_rewards = BitSet::new();
    previous_lost_rewards.insert(100);
    previous_lost_rewards.insert(101);
    previous_lost_rewards.insert(102);
    previous_lost_rewards.insert(104);

    let mut b_set = BTreeSet::new();
    b_set.insert(0);
    b_set.insert(10);
    let mut current_disabled_slots = BTreeMap::new();
    current_disabled_slots.insert(Address::from([1u8; 20]), b_set);

    let mut b_set = BTreeSet::new();
    b_set.insert(100);
    b_set.insert(101);
    b_set.insert(102);
    b_set.insert(104);
    let mut previous_disabled_slots = BTreeMap::new();
    previous_disabled_slots.insert(Address::from([2u8; 20]), b_set);

    let contract = StakingContract {
        balance: Coin::from_u64_unchecked(300_000_000),
        active_validators,
        parked_set,
        current_lost_rewards,
        previous_lost_rewards,
        current_disabled_slots,
        previous_disabled_slots,
    };

    assert_eq!(&hex::encode(contract.serialize_to_vec()), "");
}

#[test]
fn it_can_de_serialize_a_staking_contract() {
    let bytes_1: Vec<u8> = hex::decode(CONTRACT_1).unwrap();
    let contract_1: StakingContract = Deserialize::deserialize(&mut &bytes_1[..]).unwrap();

    assert_eq!(contract_1.balance, 0.try_into().unwrap());
    assert_eq!(contract_1.active_validators.len(), 0);
    assert_eq!(contract_1.parked_set.len(), 0);
    assert_eq!(contract_1.current_lost_rewards.len(), 0);
    assert_eq!(contract_1.previous_lost_rewards.len(), 0);
    assert_eq!(contract_1.current_disabled_slots.len(), 0);
    assert_eq!(contract_1.previous_disabled_slots.len(), 0);

    let mut bytes_1_out = Vec::<u8>::with_capacity(contract_1.serialized_size());
    let size_1_out = contract_1.serialize(&mut bytes_1_out).unwrap();

    assert_eq!(size_1_out, contract_1.serialized_size());
    assert_eq!(hex::encode(bytes_1_out), CONTRACT_1);

    let bytes_2: Vec<u8> = hex::decode(CONTRACT_2).unwrap();
    let contract_2: StakingContract = Deserialize::deserialize(&mut &bytes_2[..]).unwrap();

    assert_eq!(contract_2.balance, 300_000_000.try_into().unwrap());
    assert_eq!(contract_2.active_validators.len(), 1);
    assert_eq!(contract_2.parked_set.len(), 1);
    assert_eq!(contract_2.current_lost_rewards.len(), 2);
    assert_eq!(contract_2.previous_lost_rewards.len(), 4);
    assert_eq!(contract_2.current_disabled_slots.len(), 1);
    assert_eq!(contract_2.previous_disabled_slots.len(), 1);

    let mut bytes_2_out = Vec::<u8>::with_capacity(contract_2.serialized_size());
    let size_2_out = contract_2.serialize(&mut bytes_2_out).unwrap();

    assert_eq!(size_2_out, contract_2.serialized_size());
    assert_eq!(hex::encode(bytes_2_out), CONTRACT_2);
}

#[test]
fn can_get_it() {
    let env = VolatileEnvironment::new(10).unwrap();
    let accounts_tree = AccountsTrie::new(env.clone(), "AccountsTrie");
    let mut db_txn = WriteTransaction::new(&env);

    make_sample_contract(&accounts_tree, &mut db_txn, true);

    let staking_contract = StakingContract::get_staking_contract(&accounts_tree, &db_txn);

    assert_eq!(
        staking_contract.balance,
        Coin::from_u64_unchecked(150_000_000 + VALIDATOR_DEPOSIT)
    );

    let validator = StakingContract::get_validator(
        &accounts_tree,
        &db_txn,
        &Address::from_any_str(VALIDATOR_ADDRESS).unwrap(),
    )
    .unwrap();

    assert_eq!(
        validator.balance,
        Coin::from_u64_unchecked(150_000_000 + VALIDATOR_DEPOSIT)
    );

    let staker = StakingContract::get_staker(
        &accounts_tree,
        &db_txn,
        &Address::from_any_str(STAKER_ADDRESS).unwrap(),
    )
    .unwrap();

    assert_eq!(staker.balance, Coin::from_u64_unchecked(150_000_000));
}

#[test]
fn create_validator_works() {
    let env = VolatileEnvironment::new(10).unwrap();
    let accounts_tree = AccountsTrie::new(env.clone(), "AccountsTrie");
    let mut db_txn = WriteTransaction::new(&env);

    make_empty_contract(&accounts_tree, &mut db_txn);

    let cold_keypair = ed25519_key_pair(VALIDATOR_PRIVATE_KEY);

    let validator_address = Address::from_any_str(VALIDATOR_ADDRESS).unwrap();

    let signing_key =
        PublicKey::deserialize_from_vec(&hex::decode(VALIDATOR_SIGNING_KEY).unwrap()).unwrap();

    let voting_key =
        BlsPublicKey::deserialize_from_vec(&hex::decode(VALIDATOR_VOTING_KEY).unwrap()).unwrap();

    let voting_keypair = bls_key_pair(VALIDATOR_VOTING_SECRET_KEY);

    // Works in the valid case.
    let tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::CreateValidator {
            signing_key,
            voting_key: voting_key.clone(),
            proof_of_knowledge: voting_keypair
                .sign(&voting_key.serialize_to_vec())
                .compress(),
            reward_address: Address::from([3u8; 20]),
            signal_data: None,
            proof: SignatureProof::default(),
        },
        VALIDATOR_DEPOSIT,
        &cold_keypair,
    );

    assert_eq!(
        StakingContract::commit_incoming_transaction(&accounts_tree, &mut db_txn, &tx, 2, 0),
        Ok(None)
    );

    let validator =
        StakingContract::get_validator(&accounts_tree, &db_txn, &validator_address).unwrap();

    assert_eq!(validator.address, validator_address);
    assert_eq!(validator.signing_key, signing_key);
    assert_eq!(validator.voting_key, voting_key);
    assert_eq!(validator.reward_address, Address::from([3u8; 20]));
    assert_eq!(validator.signal_data, None);
    assert_eq!(
        validator.balance,
        Coin::from_u64_unchecked(VALIDATOR_DEPOSIT)
    );
    assert_eq!(validator.num_stakers, 0);
    assert_eq!(validator.inactivity_flag, None);

    let staking_contract = StakingContract::get_staking_contract(&accounts_tree, &db_txn);

    assert_eq!(
        staking_contract.balance,
        Coin::from_u64_unchecked(VALIDATOR_DEPOSIT)
    );

    assert_eq!(
        staking_contract.active_validators.get(&validator_address),
        Some(&Coin::from_u64_unchecked(VALIDATOR_DEPOSIT))
    );

    // Doesn't work when the validator already exists.
    assert_eq!(
        StakingContract::commit_incoming_transaction(&accounts_tree, &mut db_txn, &tx, 2, 0),
        Err(AccountError::AlreadyExistentAddress {
            address: validator_address.clone()
        })
    );

    // Can revert the transaction.
    assert_eq!(
        StakingContract::revert_incoming_transaction(&accounts_tree, &mut db_txn, &tx, 2, 0, None),
        Ok(())
    );

    assert_eq!(
        StakingContract::get_validator(&accounts_tree, &db_txn, &validator_address,),
        None
    );

    let staking_contract = StakingContract::get_staking_contract(&accounts_tree, &db_txn);

    assert_eq!(staking_contract.balance, Coin::ZERO);

    assert_eq!(
        staking_contract.active_validators.get(&validator_address),
        None
    );
}

#[test]
fn update_validator_works() {
    let env = VolatileEnvironment::new(10).unwrap();
    let accounts_tree = AccountsTrie::new(env.clone(), "AccountsTrie");
    let mut db_txn = WriteTransaction::new(&env);

    make_sample_contract(&accounts_tree, &mut db_txn, true);

    let validator_address = Address::from_any_str(VALIDATOR_ADDRESS).unwrap();

    let cold_keypair = ed25519_key_pair(VALIDATOR_PRIVATE_KEY);

    let new_voting_keypair = BlsKeyPair::generate_default_csprng();

    // Works in the valid case.
    let tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::UpdateValidator {
            new_signing_key: Some(PublicKey::from([88u8; 32])),
            new_voting_key: Some(new_voting_keypair.public_key.compress()),
            new_reward_address: Some(Address::from([77u8; 20])),
            new_signal_data: Some(Some(Blake2bHash::default())),
            new_proof_of_knowledge: Some(
                new_voting_keypair
                    .sign(&new_voting_keypair.public_key.serialize_to_vec())
                    .compress(),
            ),
            proof: SignatureProof::default(),
        },
        0,
        &cold_keypair,
    );

    let old_signing_key =
        PublicKey::deserialize_from_vec(&hex::decode(VALIDATOR_SIGNING_KEY).unwrap()).unwrap();

    let old_voting_key =
        BlsPublicKey::deserialize_from_vec(&hex::decode(VALIDATOR_VOTING_KEY).unwrap()).unwrap();

    let old_reward_address = Address::from_any_str(VALIDATOR_ADDRESS).unwrap();

    let receipt = UpdateValidatorReceipt {
        no_op: false,
        old_signing_key,
        old_voting_key: old_voting_key.clone(),
        old_reward_address: old_reward_address.clone(),
        old_signal_data: None,
    }
    .serialize_to_vec();

    assert_eq!(
        StakingContract::commit_incoming_transaction(&accounts_tree, &mut db_txn, &tx, 2, 0),
        Ok(Some(receipt.clone()))
    );

    let validator =
        StakingContract::get_validator(&accounts_tree, &db_txn, &validator_address).unwrap();

    assert_eq!(validator.address, validator_address);
    assert_eq!(validator.signing_key, PublicKey::from([88u8; 32]));
    assert_eq!(
        validator.voting_key,
        new_voting_keypair.public_key.compress()
    );
    assert_eq!(validator.reward_address, Address::from([77u8; 20]));
    assert_eq!(validator.signal_data, Some(Blake2bHash::default()));
    assert_eq!(
        validator.balance,
        Coin::from_u64_unchecked(VALIDATOR_DEPOSIT + 150_000_000)
    );
    assert_eq!(validator.num_stakers, 1);
    assert_eq!(validator.inactivity_flag, None);

    // Can revert the transaction.
    assert_eq!(
        StakingContract::revert_incoming_transaction(
            &accounts_tree,
            &mut db_txn,
            &tx,
            2,
            0,
            Some(&receipt)
        ),
        Ok(())
    );

    let validator =
        StakingContract::get_validator(&accounts_tree, &db_txn, &validator_address).unwrap();

    assert_eq!(validator.address, validator_address);
    assert_eq!(validator.signing_key, old_signing_key);
    assert_eq!(validator.voting_key, old_voting_key);
    assert_eq!(validator.reward_address, old_reward_address);
    assert_eq!(validator.signal_data, None);
    assert_eq!(
        validator.balance,
        Coin::from_u64_unchecked(VALIDATOR_DEPOSIT + 150_000_000)
    );
    assert_eq!(validator.num_stakers, 1);
    assert_eq!(validator.inactivity_flag, None);

    // Works when the validator doesn't exist.
    let keypair = ed25519_key_pair(STAKER_PRIVATE_KEY);

    let tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::UpdateValidator {
            new_signing_key: Some(PublicKey::from([88u8; 32])),
            new_voting_key: Some(new_voting_keypair.public_key.compress()),
            new_reward_address: Some(Address::from([77u8; 20])),
            new_signal_data: Some(Some(Blake2bHash::default())),
            new_proof_of_knowledge: Some(
                new_voting_keypair
                    .sign(&new_voting_keypair.public_key.serialize_to_vec())
                    .compress(),
            ),
            proof: SignatureProof::default(),
        },
        0,
        &keypair,
    );

    let no_op_receipt = UpdateValidatorReceipt {
        no_op: true,
        old_signing_key: Default::default(),
        old_voting_key: CompressedPublicKey::default(),
        old_reward_address: Default::default(),
        old_signal_data: None,
    }
    .serialize_to_vec();

    assert_eq!(
        StakingContract::commit_incoming_transaction(&accounts_tree, &mut db_txn, &tx, 2, 0),
        Ok(Some(no_op_receipt))
    );
}

#[test]
fn inactivate_validator_works() {
    let env = VolatileEnvironment::new(10).unwrap();
    let accounts_tree = AccountsTrie::new(env.clone(), "AccountsTrie");
    let mut db_txn = WriteTransaction::new(&env);

    make_sample_contract(&accounts_tree, &mut db_txn, true);

    let validator_address = Address::from_any_str(VALIDATOR_ADDRESS).unwrap();

    let cold_keypair = ed25519_key_pair(VALIDATOR_PRIVATE_KEY);

    let signing_key =
        PublicKey::deserialize_from_vec(&hex::decode(VALIDATOR_SIGNING_KEY).unwrap()).unwrap();

    let signing_keypair = ed25519_key_pair(VALIDATOR_SIGNING_SECRET_KEY);

    let voting_key =
        BlsPublicKey::deserialize_from_vec(&hex::decode(VALIDATOR_VOTING_KEY).unwrap()).unwrap();

    // First, park the validator.
    let mut staking_contract = StakingContract::get_staking_contract(&accounts_tree, &db_txn);

    staking_contract
        .parked_set
        .insert(validator_address.clone());

    accounts_tree.put(
        &mut db_txn,
        &StakingContract::get_key_staking_contract(),
        Account::Staking(staking_contract),
    );

    // Works in the valid case.
    let tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::InactivateValidator {
            validator_address: validator_address.clone(),
            proof: SignatureProof::default(),
        },
        0,
        &signing_keypair,
    );

    let receipt = InactivateValidatorReceipt {
        no_op: false,
        parked_set: true,
    }
    .serialize_to_vec();

    assert_eq!(
        StakingContract::commit_incoming_transaction(&accounts_tree, &mut db_txn, &tx, 2, 0),
        Ok(Some(receipt.clone()))
    );

    let validator =
        StakingContract::get_validator(&accounts_tree, &db_txn, &validator_address).unwrap();

    assert_eq!(validator.address, validator_address);
    assert_eq!(validator.signing_key, signing_key);
    assert_eq!(validator.voting_key, voting_key);
    assert_eq!(validator.reward_address, validator_address);
    assert_eq!(validator.signal_data, None);
    assert_eq!(
        validator.balance,
        Coin::from_u64_unchecked(VALIDATOR_DEPOSIT + 150_000_000)
    );
    assert_eq!(validator.num_stakers, 1);
    assert_eq!(validator.inactivity_flag, Some(2));

    let staking_contract = StakingContract::get_staking_contract(&accounts_tree, &db_txn);

    assert!(!staking_contract
        .active_validators
        .contains_key(&validator_address));
    assert!(!staking_contract.parked_set.contains(&validator_address));

    // Try with an already inactive validator.
    let tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::InactivateValidator {
            validator_address: validator_address.clone(),
            proof: SignatureProof::default(),
        },
        0,
        &signing_keypair,
    );

    let no_op_receipt = InactivateValidatorReceipt {
        no_op: true,
        parked_set: false,
    }
    .serialize_to_vec();

    assert_eq!(
        StakingContract::commit_incoming_transaction(&accounts_tree, &mut db_txn, &tx, 2, 0),
        Ok(Some(no_op_receipt.clone()))
    );

    // Try with a wrong signature.
    let tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::InactivateValidator {
            validator_address: validator_address.clone(),
            proof: SignatureProof::default(),
        },
        0,
        &cold_keypair,
    );

    assert_eq!(
        StakingContract::commit_incoming_transaction(&accounts_tree, &mut db_txn, &tx, 2, 0),
        Ok(Some(no_op_receipt.clone()))
    );

    // Can revert the transaction.
    assert_eq!(
        StakingContract::revert_incoming_transaction(
            &accounts_tree,
            &mut db_txn,
            &tx,
            2,
            0,
            Some(&receipt)
        ),
        Ok(())
    );

    let validator =
        StakingContract::get_validator(&accounts_tree, &db_txn, &validator_address).unwrap();

    assert_eq!(validator.address, validator_address);
    assert_eq!(validator.signing_key, signing_key);
    assert_eq!(validator.voting_key, voting_key);
    assert_eq!(validator.reward_address, validator_address);
    assert_eq!(validator.signal_data, None);
    assert_eq!(
        validator.balance,
        Coin::from_u64_unchecked(VALIDATOR_DEPOSIT + 150_000_000)
    );
    assert_eq!(validator.num_stakers, 1);
    assert_eq!(validator.inactivity_flag, None);

    let staking_contract = StakingContract::get_staking_contract(&accounts_tree, &db_txn);

    assert!(staking_contract
        .active_validators
        .contains_key(&validator_address));
    assert!(staking_contract.parked_set.contains(&validator_address));

    // Works when the validator doesn't exist.
    let fake_address = Address::from_any_str(STAKER_ADDRESS).unwrap();

    let tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::InactivateValidator {
            validator_address: fake_address,
            proof: SignatureProof::default(),
        },
        0,
        &signing_keypair,
    );

    assert_eq!(
        StakingContract::commit_incoming_transaction(&accounts_tree, &mut db_txn, &tx, 2, 0),
        Ok(Some(no_op_receipt))
    );
}

#[test]
fn reactivate_validator_works() {
    let env = VolatileEnvironment::new(10).unwrap();
    let accounts_tree = AccountsTrie::new(env.clone(), "AccountsTrie");
    let mut db_txn = WriteTransaction::new(&env);

    make_sample_contract(&accounts_tree, &mut db_txn, true);

    let validator_address = Address::from_any_str(VALIDATOR_ADDRESS).unwrap();

    let cold_keypair = ed25519_key_pair(VALIDATOR_PRIVATE_KEY);

    let signing_key =
        PublicKey::deserialize_from_vec(&hex::decode(VALIDATOR_SIGNING_KEY).unwrap()).unwrap();

    let signing_keypair = ed25519_key_pair(VALIDATOR_SIGNING_SECRET_KEY);

    let voting_key =
        BlsPublicKey::deserialize_from_vec(&hex::decode(VALIDATOR_VOTING_KEY).unwrap()).unwrap();

    // To begin with, retire the validator.
    let tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::InactivateValidator {
            validator_address: validator_address.clone(),
            proof: SignatureProof::default(),
        },
        0,
        &signing_keypair,
    );

    StakingContract::commit_incoming_transaction(&accounts_tree, &mut db_txn, &tx, 2, 0).unwrap();

    // Works in the valid case.
    let tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::ReactivateValidator {
            validator_address: validator_address.clone(),
            proof: SignatureProof::default(),
        },
        0,
        &signing_keypair,
    );

    let receipt = ReactivateValidatorReceipt {
        no_op: false,
        retire_time: 2,
    }
    .serialize_to_vec();

    assert_eq!(
        StakingContract::commit_incoming_transaction(&accounts_tree, &mut db_txn, &tx, 2, 0),
        Ok(Some(receipt.clone()))
    );

    let validator =
        StakingContract::get_validator(&accounts_tree, &db_txn, &validator_address).unwrap();

    assert_eq!(validator.address, validator_address);
    assert_eq!(validator.signing_key, signing_key);
    assert_eq!(validator.voting_key, voting_key);
    assert_eq!(validator.reward_address, validator_address);
    assert_eq!(validator.signal_data, None);
    assert_eq!(
        validator.balance,
        Coin::from_u64_unchecked(VALIDATOR_DEPOSIT + 150_000_000)
    );
    assert_eq!(validator.num_stakers, 1);
    assert_eq!(validator.inactivity_flag, None);

    let staking_contract = StakingContract::get_staking_contract(&accounts_tree, &db_txn);

    assert_eq!(
        staking_contract.active_validators.get(&validator_address),
        Some(&Coin::from_u64_unchecked(VALIDATOR_DEPOSIT + 150_000_000))
    );

    // Try with an already active validator.
    let tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::ReactivateValidator {
            validator_address: validator_address.clone(),
            proof: SignatureProof::default(),
        },
        0,
        &signing_keypair,
    );

    let no_op_receipt = ReactivateValidatorReceipt {
        no_op: true,
        retire_time: 0,
    }
    .serialize_to_vec();

    assert_eq!(
        StakingContract::commit_incoming_transaction(&accounts_tree, &mut db_txn, &tx, 2, 0),
        Ok(Some(no_op_receipt.clone()))
    );

    // Try with a wrong signature.
    let tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::ReactivateValidator {
            validator_address: validator_address.clone(),
            proof: SignatureProof::default(),
        },
        0,
        &cold_keypair,
    );

    assert_eq!(
        StakingContract::commit_incoming_transaction(&accounts_tree, &mut db_txn, &tx, 2, 0),
        Ok(Some(no_op_receipt.clone()))
    );

    // Can revert the transaction.
    assert_eq!(
        StakingContract::revert_incoming_transaction(
            &accounts_tree,
            &mut db_txn,
            &tx,
            2,
            0,
            Some(&receipt)
        ),
        Ok(())
    );

    let validator =
        StakingContract::get_validator(&accounts_tree, &db_txn, &validator_address).unwrap();

    assert_eq!(validator.address, validator_address);
    assert_eq!(validator.signing_key, signing_key);
    assert_eq!(validator.voting_key, voting_key);
    assert_eq!(validator.reward_address, validator_address);
    assert_eq!(validator.signal_data, None);
    assert_eq!(
        validator.balance,
        Coin::from_u64_unchecked(VALIDATOR_DEPOSIT + 150_000_000)
    );
    assert_eq!(validator.num_stakers, 1);
    assert_eq!(validator.inactivity_flag, Some(2));

    let staking_contract = StakingContract::get_staking_contract(&accounts_tree, &db_txn);

    assert!(!staking_contract
        .active_validators
        .contains_key(&validator_address));

    // Works when the validator doesn't exist.
    let fake_address = Address::from_any_str(STAKER_ADDRESS).unwrap();

    let tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::ReactivateValidator {
            validator_address: fake_address,
            proof: SignatureProof::default(),
        },
        0,
        &cold_keypair,
    );

    assert_eq!(
        StakingContract::commit_incoming_transaction(&accounts_tree, &mut db_txn, &tx, 2, 0),
        Ok(Some(no_op_receipt))
    );
}

#[test]
fn unpark_validator_works() {
    let env = VolatileEnvironment::new(10).unwrap();
    let accounts_tree = AccountsTrie::new(env.clone(), "AccountsTrie");
    let mut db_txn = WriteTransaction::new(&env);

    make_sample_contract(&accounts_tree, &mut db_txn, true);

    let validator_address = Address::from_any_str(VALIDATOR_ADDRESS).unwrap();

    let cold_keypair = ed25519_key_pair(VALIDATOR_PRIVATE_KEY);

    let signing_keypair = ed25519_key_pair(VALIDATOR_SIGNING_SECRET_KEY);

    // To begin with, unpark and disable the validator.
    let mut staking_contract = StakingContract::get_staking_contract(&accounts_tree, &db_txn);

    let mut slots = BTreeSet::new();
    slots.insert(1);
    slots.insert(2);
    slots.insert(3);

    staking_contract
        .parked_set
        .insert(validator_address.clone());
    staking_contract
        .current_disabled_slots
        .insert(validator_address.clone(), slots.clone());
    staking_contract
        .previous_disabled_slots
        .insert(validator_address.clone(), slots.clone());

    accounts_tree.put(
        &mut db_txn,
        &StakingContract::get_key_staking_contract(),
        Account::Staking(staking_contract),
    );

    // Works in the valid case.
    let tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::UnparkValidator {
            validator_address: validator_address.clone(),
            proof: SignatureProof::default(),
        },
        0,
        &signing_keypair,
    );

    let receipt = UnparkValidatorReceipt {
        no_op: false,
        parked_set: true,
        current_disabled_slots: Some(slots.clone()),
        previous_disabled_slots: Some(slots),
    }
    .serialize_to_vec();

    assert_eq!(
        StakingContract::commit_incoming_transaction(&accounts_tree, &mut db_txn, &tx, 2, 0),
        Ok(Some(receipt.clone()))
    );

    let staking_contract = StakingContract::get_staking_contract(&accounts_tree, &db_txn);

    assert!(!staking_contract.parked_set.contains(&validator_address));
    assert!(!staking_contract
        .current_disabled_slots
        .contains_key(&validator_address));
    assert!(!staking_contract
        .previous_disabled_slots
        .contains_key(&validator_address));

    // Try with an already unparked validator.
    let tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::UnparkValidator {
            validator_address: validator_address.clone(),
            proof: SignatureProof::default(),
        },
        0,
        &signing_keypair,
    );

    let no_op_receipt = UnparkValidatorReceipt {
        no_op: true,
        parked_set: false,
        current_disabled_slots: None,
        previous_disabled_slots: None,
    }
    .serialize_to_vec();

    assert_eq!(
        StakingContract::commit_incoming_transaction(&accounts_tree, &mut db_txn, &tx, 2, 0),
        Ok(Some(no_op_receipt.clone()))
    );

    // Try with a wrong signature.
    let tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::UnparkValidator {
            validator_address: validator_address.clone(),
            proof: SignatureProof::default(),
        },
        0,
        &cold_keypair,
    );

    assert_eq!(
        StakingContract::commit_incoming_transaction(&accounts_tree, &mut db_txn, &tx, 2, 0),
        Ok(Some(no_op_receipt.clone()))
    );

    // Can revert the transaction.
    assert_eq!(
        StakingContract::revert_incoming_transaction(
            &accounts_tree,
            &mut db_txn,
            &tx,
            2,
            0,
            Some(&receipt)
        ),
        Ok(())
    );

    let staking_contract = StakingContract::get_staking_contract(&accounts_tree, &db_txn);

    assert!(staking_contract.parked_set.contains(&validator_address));
    assert!(staking_contract
        .current_disabled_slots
        .contains_key(&validator_address));
    assert!(staking_contract
        .previous_disabled_slots
        .contains_key(&validator_address));

    // Works when the validator doesn't exist.
    let fake_address = Address::from_any_str(STAKER_ADDRESS).unwrap();

    let tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::UnparkValidator {
            validator_address: fake_address,
            proof: SignatureProof::default(),
        },
        0,
        &cold_keypair,
    );

    assert_eq!(
        StakingContract::commit_incoming_transaction(&accounts_tree, &mut db_txn, &tx, 2, 0),
        Ok(Some(no_op_receipt))
    );
}

#[test]
fn delete_validator_works() {
    let env = VolatileEnvironment::new(10).unwrap();
    let accounts_tree = AccountsTrie::new(env.clone(), "AccountsTrie");
    let mut db_txn = WriteTransaction::new(&env);

    make_sample_contract(&accounts_tree, &mut db_txn, true);

    // Doesn't work when the validator is still active.
    let tx = make_delete_validator_transaction();

    assert_eq!(
        StakingContract::commit_outgoing_transaction(&accounts_tree, &mut db_txn, &tx, 2, 0),
        Err(AccountError::InvalidForSender)
    );

    // Retire the validator.
    let retire_tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::InactivateValidator {
            validator_address: Address::from_any_str(VALIDATOR_ADDRESS).unwrap(),
            proof: SignatureProof::default(),
        },
        0,
        &ed25519_key_pair(VALIDATOR_SIGNING_SECRET_KEY),
    );

    StakingContract::commit_incoming_transaction(&accounts_tree, &mut db_txn, &retire_tx, 2, 0)
        .unwrap();

    // Doesn't work until the next election block.
    let next_election_block = policy::election_block_after(2);

    for h in 2..=next_election_block {
        assert_eq!(
            StakingContract::commit_outgoing_transaction(&accounts_tree, &mut db_txn, &tx, h, 0),
            Err(AccountError::InvalidForSender)
        );
    }

    // Works in the valid case.
    let validator_address = Address::from_any_str(VALIDATOR_ADDRESS).unwrap();

    let signing_key =
        PublicKey::deserialize_from_vec(&hex::decode(VALIDATOR_SIGNING_KEY).unwrap()).unwrap();

    let voting_key =
        BlsPublicKey::deserialize_from_vec(&hex::decode(VALIDATOR_VOTING_KEY).unwrap()).unwrap();

    let reward_address = Address::from_any_str(VALIDATOR_ADDRESS).unwrap();

    let staker_address = Address::from_any_str(STAKER_ADDRESS).unwrap();

    let receipt = DeleteValidatorReceipt {
        signing_key,
        voting_key: voting_key.clone(),
        reward_address: reward_address.clone(),
        signal_data: None,
        retire_time: 2,
        stakers: vec![staker_address.clone()],
    }
    .serialize_to_vec();

    assert_eq!(
        StakingContract::commit_outgoing_transaction(
            &accounts_tree,
            &mut db_txn,
            &tx,
            next_election_block + BLOCKS_PER_BATCH + 1,
            0
        ),
        Ok(Some(receipt.clone()))
    );

    assert_eq!(
        StakingContract::get_validator(&accounts_tree, &db_txn, &validator_address),
        None
    );

    assert_eq!(
        accounts_tree.get(
            &db_txn,
            &StakingContract::get_key_validator_staker(&validator_address, &staker_address)
        ),
        None
    );

    let staker = StakingContract::get_staker(&accounts_tree, &db_txn, &staker_address).unwrap();

    assert_eq!(staker.delegation, None);

    let staking_contract = StakingContract::get_staking_contract(&accounts_tree, &db_txn);

    assert_eq!(
        staking_contract.balance,
        Coin::from_u64_unchecked(150_000_000)
    );

    // Can revert the transaction.
    assert_eq!(
        StakingContract::revert_outgoing_transaction(
            &accounts_tree,
            &mut db_txn,
            &tx,
            next_election_block + 1,
            0,
            Some(&receipt)
        ),
        Ok(())
    );

    let validator = StakingContract::get_validator(
        &accounts_tree,
        &db_txn,
        &Address::from_any_str(VALIDATOR_ADDRESS).unwrap(),
    )
    .unwrap();

    assert_eq!(validator.address, validator_address);
    assert_eq!(validator.signing_key, signing_key);
    assert_eq!(validator.voting_key, voting_key);
    assert_eq!(validator.reward_address, reward_address);
    assert_eq!(validator.signal_data, None);
    assert_eq!(
        validator.balance,
        Coin::from_u64_unchecked(VALIDATOR_DEPOSIT + 150_000_000)
    );
    assert_eq!(validator.num_stakers, 1);
    assert_eq!(validator.inactivity_flag, Some(2));

    assert_eq!(
        accounts_tree.get(
            &db_txn,
            &StakingContract::get_key_validator_staker(&validator_address, &staker_address)
        ),
        Some(Account::StakingValidatorsStaker(staker_address.clone()))
    );

    let staker = StakingContract::get_staker(&accounts_tree, &db_txn, &staker_address).unwrap();

    assert_eq!(staker.delegation, Some(validator_address));

    let staking_contract = StakingContract::get_staking_contract(&accounts_tree, &db_txn);

    assert_eq!(
        staking_contract.balance,
        Coin::from_u64_unchecked(VALIDATOR_DEPOSIT + 150_000_000)
    );
}

#[test]
fn create_staker_works() {
    let env = VolatileEnvironment::new(10).unwrap();
    let accounts_tree = AccountsTrie::new(env.clone(), "AccountsTrie");
    let mut db_txn = WriteTransaction::new(&env);

    make_sample_contract(&accounts_tree, &mut db_txn, false);

    let staker_keypair = ed25519_key_pair(STAKER_PRIVATE_KEY);

    let staker_address = Address::from_any_str(STAKER_ADDRESS).unwrap();

    let validator_address = Address::from_any_str(VALIDATOR_ADDRESS).unwrap();

    // Works in the valid case.
    let tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::CreateStaker {
            delegation: Some(validator_address.clone()),
            proof: SignatureProof::default(),
        },
        150_000_000,
        &staker_keypair,
    );

    assert_eq!(
        StakingContract::commit_incoming_transaction(&accounts_tree, &mut db_txn, &tx, 2, 0),
        Ok(None)
    );

    let staker = StakingContract::get_staker(&accounts_tree, &db_txn, &staker_address).unwrap();

    assert_eq!(staker.address, staker_address);
    assert_eq!(staker.balance, Coin::from_u64_unchecked(150_000_000));
    assert_eq!(staker.delegation, Some(validator_address.clone()));

    let validator =
        StakingContract::get_validator(&accounts_tree, &db_txn, &validator_address).unwrap();

    assert_eq!(
        validator.balance,
        Coin::from_u64_unchecked(VALIDATOR_DEPOSIT + 150_000_000)
    );
    assert_eq!(validator.num_stakers, 1);

    assert_eq!(
        accounts_tree.get(
            &db_txn,
            &StakingContract::get_key_validator_staker(&validator_address, &staker_address)
        ),
        Some(Account::StakingValidatorsStaker(staker_address.clone()))
    );

    let staking_contract = StakingContract::get_staking_contract(&accounts_tree, &db_txn);

    assert_eq!(
        staking_contract.balance,
        Coin::from_u64_unchecked(VALIDATOR_DEPOSIT + 150_000_000)
    );

    assert_eq!(
        staking_contract.active_validators.get(&validator_address),
        Some(&Coin::from_u64_unchecked(VALIDATOR_DEPOSIT + 150_000_000))
    );

    // Doesn't work when the staker already exists.
    assert_eq!(
        StakingContract::commit_incoming_transaction(&accounts_tree, &mut db_txn, &tx, 2, 0),
        Err(AccountError::AlreadyExistentAddress {
            address: staker_address.clone()
        })
    );

    // Can revert the transaction.
    assert_eq!(
        StakingContract::revert_incoming_transaction(&accounts_tree, &mut db_txn, &tx, 2, 0, None),
        Ok(())
    );

    assert_eq!(
        StakingContract::get_staker(&accounts_tree, &db_txn, &staker_address),
        None
    );

    let validator =
        StakingContract::get_validator(&accounts_tree, &db_txn, &validator_address).unwrap();

    assert_eq!(
        validator.balance,
        Coin::from_u64_unchecked(VALIDATOR_DEPOSIT)
    );
    assert_eq!(validator.num_stakers, 0);

    assert_eq!(
        accounts_tree.get(
            &db_txn,
            &StakingContract::get_key_validator_staker(&validator_address, &staker_address)
        ),
        None
    );

    let staking_contract = StakingContract::get_staking_contract(&accounts_tree, &db_txn);

    assert_eq!(
        staking_contract.balance,
        Coin::from_u64_unchecked(VALIDATOR_DEPOSIT)
    );

    assert_eq!(
        staking_contract.active_validators.get(&validator_address),
        Some(&Coin::from_u64_unchecked(VALIDATOR_DEPOSIT))
    );
}

#[test]
fn stake_works() {
    let env = VolatileEnvironment::new(10).unwrap();
    let accounts_tree = AccountsTrie::new(env.clone(), "AccountsTrie");
    let mut db_txn = WriteTransaction::new(&env);

    make_sample_contract(&accounts_tree, &mut db_txn, true);

    let staker_keypair = ed25519_key_pair(STAKER_PRIVATE_KEY);

    let staker_address = Address::from_any_str(STAKER_ADDRESS).unwrap();

    let validator_address = Address::from_any_str(VALIDATOR_ADDRESS).unwrap();

    // Works in the valid case.
    let tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::Stake {
            staker_address: staker_address.clone(),
        },
        150_000_000,
        &staker_keypair,
    );

    assert_eq!(
        StakingContract::commit_incoming_transaction(&accounts_tree, &mut db_txn, &tx, 2, 0),
        Ok(None)
    );

    let staker = StakingContract::get_staker(&accounts_tree, &db_txn, &staker_address).unwrap();

    assert_eq!(staker.address, staker_address);
    assert_eq!(staker.balance, Coin::from_u64_unchecked(300_000_000));
    assert_eq!(staker.delegation, Some(validator_address.clone()));

    let validator =
        StakingContract::get_validator(&accounts_tree, &db_txn, &validator_address).unwrap();

    assert_eq!(
        validator.balance,
        Coin::from_u64_unchecked(VALIDATOR_DEPOSIT + 300_000_000)
    );

    let staking_contract = StakingContract::get_staking_contract(&accounts_tree, &db_txn);

    assert_eq!(
        staking_contract.balance,
        Coin::from_u64_unchecked(VALIDATOR_DEPOSIT + 300_000_000)
    );

    assert_eq!(
        staking_contract.active_validators.get(&validator_address),
        Some(&Coin::from_u64_unchecked(VALIDATOR_DEPOSIT + 300_000_000))
    );

    // Can revert the transaction.
    assert_eq!(
        StakingContract::revert_incoming_transaction(&accounts_tree, &mut db_txn, &tx, 2, 0, None),
        Ok(())
    );

    let staker = StakingContract::get_staker(&accounts_tree, &db_txn, &staker_address).unwrap();

    assert_eq!(staker.address, staker_address);
    assert_eq!(staker.balance, Coin::from_u64_unchecked(150_000_000));
    assert_eq!(staker.delegation, Some(validator_address.clone()));

    let validator =
        StakingContract::get_validator(&accounts_tree, &db_txn, &validator_address).unwrap();

    assert_eq!(
        validator.balance,
        Coin::from_u64_unchecked(VALIDATOR_DEPOSIT + 150_000_000)
    );

    let staking_contract = StakingContract::get_staking_contract(&accounts_tree, &db_txn);

    assert_eq!(
        staking_contract.balance,
        Coin::from_u64_unchecked(VALIDATOR_DEPOSIT + 150_000_000)
    );

    assert_eq!(
        staking_contract.active_validators.get(&validator_address),
        Some(&Coin::from_u64_unchecked(VALIDATOR_DEPOSIT + 150_000_000))
    );
}

#[test]
fn update_staker_works() {
    let env = VolatileEnvironment::new(10).unwrap();
    let accounts_tree = AccountsTrie::new(env.clone(), "AccountsTrie");
    let mut db_txn = WriteTransaction::new(&env);

    make_sample_contract(&accounts_tree, &mut db_txn, true);

    let staker_keypair = ed25519_key_pair(STAKER_PRIVATE_KEY);

    let staker_address = Address::from_any_str(STAKER_ADDRESS).unwrap();

    let validator_address = Address::from_any_str(VALIDATOR_ADDRESS).unwrap();

    let other_validator_address = Address::from([69u8; 20]);

    let signing_key =
        PublicKey::deserialize_from_vec(&hex::decode(VALIDATOR_SIGNING_KEY).unwrap()).unwrap();
    let voting_key =
        BlsPublicKey::deserialize_from_vec(&hex::decode(VALIDATOR_VOTING_KEY).unwrap()).unwrap();

    // To begin with, add another validator.
    StakingContract::create_validator(
        &accounts_tree,
        &mut db_txn,
        &other_validator_address,
        signing_key,
        voting_key,
        other_validator_address.clone(),
        None,
    )
    .unwrap();

    // Works when changing to another validator.
    let tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::UpdateStaker {
            new_delegation: Some(other_validator_address.clone()),
            proof: SignatureProof::default(),
        },
        0,
        &staker_keypair,
    );

    let receipt = StakerReceipt {
        no_op: false,
        delegation: Some(validator_address.clone()),
    }
    .serialize_to_vec();

    assert_eq!(
        StakingContract::commit_incoming_transaction(&accounts_tree, &mut db_txn, &tx, 2, 0),
        Ok(Some(receipt))
    );

    let staker = StakingContract::get_staker(&accounts_tree, &db_txn, &staker_address).unwrap();

    assert_eq!(staker.address, staker_address);
    assert_eq!(staker.balance, Coin::from_u64_unchecked(150_000_000));
    assert_eq!(staker.delegation, Some(other_validator_address.clone()));

    let old_validator =
        StakingContract::get_validator(&accounts_tree, &db_txn, &validator_address).unwrap();

    assert_eq!(
        old_validator.balance,
        Coin::from_u64_unchecked(VALIDATOR_DEPOSIT)
    );
    assert_eq!(old_validator.num_stakers, 0);

    let new_validator =
        StakingContract::get_validator(&accounts_tree, &db_txn, &other_validator_address).unwrap();

    assert_eq!(
        new_validator.balance,
        Coin::from_u64_unchecked(VALIDATOR_DEPOSIT + 150_000_000)
    );
    assert_eq!(new_validator.num_stakers, 1);

    assert_eq!(
        accounts_tree.get(
            &db_txn,
            &StakingContract::get_key_validator_staker(&other_validator_address, &staker_address)
        ),
        Some(Account::StakingValidatorsStaker(staker_address.clone()))
    );

    let staking_contract = StakingContract::get_staking_contract(&accounts_tree, &db_txn);

    assert_eq!(
        staking_contract.active_validators.get(&validator_address),
        Some(&Coin::from_u64_unchecked(VALIDATOR_DEPOSIT))
    );

    assert_eq!(
        staking_contract
            .active_validators
            .get(&other_validator_address),
        Some(&Coin::from_u64_unchecked(VALIDATOR_DEPOSIT + 150_000_000))
    );

    // Doesn't work when the staker doesn't exist.
    let tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::UpdateStaker {
            new_delegation: Some(staker_address.clone()),
            proof: SignatureProof::default(),
        },
        0,
        &staker_keypair,
    );

    let no_op_receipt = StakerReceipt {
        no_op: true,
        delegation: None,
    }
    .serialize_to_vec();

    assert_eq!(
        StakingContract::commit_incoming_transaction(&accounts_tree, &mut db_txn, &tx, 2, 0),
        Ok(Some(no_op_receipt.clone()))
    );

    // Works when changing to no validator.
    let tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::UpdateStaker {
            new_delegation: None,
            proof: SignatureProof::default(),
        },
        0,
        &staker_keypair,
    );

    let receipt = StakerReceipt {
        no_op: false,
        delegation: Some(other_validator_address.clone()),
    }
    .serialize_to_vec();

    assert_eq!(
        StakingContract::commit_incoming_transaction(&accounts_tree, &mut db_txn, &tx, 2, 0),
        Ok(Some(receipt.clone()))
    );

    let staker = StakingContract::get_staker(&accounts_tree, &db_txn, &staker_address).unwrap();

    assert_eq!(staker.address, staker_address);
    assert_eq!(staker.balance, Coin::from_u64_unchecked(150_000_000));
    assert_eq!(staker.delegation, None);

    let other_validator =
        StakingContract::get_validator(&accounts_tree, &db_txn, &other_validator_address).unwrap();

    assert_eq!(
        other_validator.balance,
        Coin::from_u64_unchecked(VALIDATOR_DEPOSIT)
    );
    assert_eq!(other_validator.num_stakers, 0);

    let staking_contract = StakingContract::get_staking_contract(&accounts_tree, &db_txn);

    assert_eq!(
        staking_contract
            .active_validators
            .get(&other_validator_address),
        Some(&Coin::from_u64_unchecked(VALIDATOR_DEPOSIT))
    );

    // Can revert the transaction.
    assert_eq!(
        StakingContract::revert_incoming_transaction(
            &accounts_tree,
            &mut db_txn,
            &tx,
            2,
            0,
            Some(&receipt)
        ),
        Ok(())
    );

    let validator =
        StakingContract::get_validator(&accounts_tree, &db_txn, &other_validator_address).unwrap();

    assert_eq!(
        validator.balance,
        Coin::from_u64_unchecked(VALIDATOR_DEPOSIT + 150_000_000)
    );
    assert_eq!(validator.num_stakers, 1);

    assert_eq!(
        accounts_tree.get(
            &db_txn,
            &StakingContract::get_key_validator_staker(&other_validator_address, &staker_address)
        ),
        Some(Account::StakingValidatorsStaker(staker_address.clone()))
    );

    let staking_contract = StakingContract::get_staking_contract(&accounts_tree, &db_txn);

    assert_eq!(
        staking_contract
            .active_validators
            .get(&other_validator_address),
        Some(&Coin::from_u64_unchecked(VALIDATOR_DEPOSIT + 150_000_000))
    );

    // Works when the staker doesn't exist.
    let keypair = ed25519_key_pair(VALIDATOR_PRIVATE_KEY);

    let tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::UpdateStaker {
            new_delegation: None,
            proof: SignatureProof::default(),
        },
        0,
        &keypair,
    );

    assert_eq!(
        StakingContract::commit_incoming_transaction(&accounts_tree, &mut db_txn, &tx, 2, 0),
        Ok(Some(no_op_receipt))
    );
}

#[test]
fn unstake_works() {
    let env = VolatileEnvironment::new(10).unwrap();
    let accounts_tree = AccountsTrie::new(env.clone(), "AccountsTrie");
    let mut db_txn = WriteTransaction::new(&env);

    make_sample_contract(&accounts_tree, &mut db_txn, true);

    // Doesn't work if the value is greater than the balance.
    let tx = make_unstake_transaction(200_000_000);

    assert_eq!(
        StakingContract::commit_outgoing_transaction(&accounts_tree, &mut db_txn, &tx, 1, 0),
        Err(AccountError::InsufficientFunds {
            needed: Coin::from_u64_unchecked(200_000_000),
            balance: Coin::from_u64_unchecked(150_000_000)
        })
    );

    // Works in the valid case.
    let tx = make_unstake_transaction(100_000_000);

    let staker_address = Address::from_any_str(STAKER_ADDRESS).unwrap();

    let validator_address = Address::from_any_str(VALIDATOR_ADDRESS).unwrap();

    assert_eq!(
        StakingContract::commit_outgoing_transaction(&accounts_tree, &mut db_txn, &tx, 1, 0),
        Ok(None)
    );

    let staker = StakingContract::get_staker(&accounts_tree, &db_txn, &staker_address).unwrap();

    assert_eq!(staker.address, staker_address);
    assert_eq!(staker.balance, Coin::from_u64_unchecked(50_000_000));
    assert_eq!(staker.delegation, Some(validator_address.clone()));

    let validator =
        StakingContract::get_validator(&accounts_tree, &db_txn, &validator_address).unwrap();

    assert_eq!(
        validator.balance,
        Coin::from_u64_unchecked(VALIDATOR_DEPOSIT + 50_000_000)
    );
    assert_eq!(validator.num_stakers, 1);
    assert_eq!(
        accounts_tree.get(
            &db_txn,
            &StakingContract::get_key_validator_staker(&validator_address, &staker_address)
        ),
        Some(Account::StakingValidatorsStaker(staker_address.clone()))
    );

    let staking_contract = StakingContract::get_staking_contract(&accounts_tree, &db_txn);

    assert_eq!(
        staking_contract.balance,
        Coin::from_u64_unchecked(VALIDATOR_DEPOSIT + 50_000_000)
    );

    assert_eq!(
        staking_contract.active_validators.get(&validator_address),
        Some(&Coin::from_u64_unchecked(VALIDATOR_DEPOSIT + 50_000_000))
    );

    // Works when removing the entire balance.
    let tx = make_unstake_transaction(50_000_000);

    let staker_address = Address::from_any_str(STAKER_ADDRESS).unwrap();

    let validator_address = Address::from_any_str(VALIDATOR_ADDRESS).unwrap();

    let receipt = StakerReceipt {
        no_op: false,
        delegation: Some(validator_address.clone()),
    }
    .serialize_to_vec();

    assert_eq!(
        StakingContract::commit_outgoing_transaction(&accounts_tree, &mut db_txn, &tx, 2, 0),
        Ok(Some(receipt.clone()))
    );

    assert_eq!(
        StakingContract::get_staker(&accounts_tree, &db_txn, &staker_address),
        None
    );

    let validator =
        StakingContract::get_validator(&accounts_tree, &db_txn, &validator_address).unwrap();

    assert_eq!(
        validator.balance,
        Coin::from_u64_unchecked(VALIDATOR_DEPOSIT)
    );
    assert_eq!(validator.num_stakers, 0);
    assert_eq!(
        accounts_tree.get(
            &db_txn,
            &StakingContract::get_key_validator_staker(&validator_address, &staker_address)
        ),
        None
    );

    let staking_contract = StakingContract::get_staking_contract(&accounts_tree, &db_txn);

    assert_eq!(
        staking_contract.balance,
        Coin::from_u64_unchecked(VALIDATOR_DEPOSIT)
    );

    assert_eq!(
        staking_contract.active_validators.get(&validator_address),
        Some(&Coin::from_u64_unchecked(VALIDATOR_DEPOSIT))
    );

    // Can revert the transaction.
    assert_eq!(
        StakingContract::revert_outgoing_transaction(
            &accounts_tree,
            &mut db_txn,
            &tx,
            2,
            0,
            Some(&receipt)
        ),
        Ok(())
    );

    let staker = StakingContract::get_staker(&accounts_tree, &db_txn, &staker_address).unwrap();

    assert_eq!(staker.address, staker_address);
    assert_eq!(staker.balance, Coin::from_u64_unchecked(50_000_000));
    assert_eq!(staker.delegation, Some(validator_address.clone()));

    let validator =
        StakingContract::get_validator(&accounts_tree, &db_txn, &validator_address).unwrap();

    assert_eq!(
        validator.balance,
        Coin::from_u64_unchecked(VALIDATOR_DEPOSIT + 50_000_000)
    );
    assert_eq!(validator.num_stakers, 1);
    assert_eq!(
        accounts_tree.get(
            &db_txn,
            &StakingContract::get_key_validator_staker(&validator_address, &staker_address)
        ),
        Some(Account::StakingValidatorsStaker(staker_address.clone()))
    );

    let staking_contract = StakingContract::get_staking_contract(&accounts_tree, &db_txn);

    assert_eq!(
        staking_contract.balance,
        Coin::from_u64_unchecked(VALIDATOR_DEPOSIT + 50_000_000)
    );

    assert_eq!(
        staking_contract.active_validators.get(&validator_address),
        Some(&Coin::from_u64_unchecked(VALIDATOR_DEPOSIT + 50_000_000))
    );
}

#[test]
fn zero_value_inherents_not_allowed() {
    let env = VolatileEnvironment::new(10).unwrap();
    let accounts_tree = AccountsTrie::new(env.clone(), "AccountsTrie");
    let mut db_txn = WriteTransaction::new(&env);

    make_sample_contract(&accounts_tree, &mut db_txn, true);

    let validator_address = Address::from_any_str(VALIDATOR_ADDRESS).unwrap();

    let inherent = Inherent {
        ty: InherentType::Slash,
        target: validator_address,
        value: Coin::ZERO,
        data: vec![],
    };

    assert_eq!(
        StakingContract::commit_inherent(&accounts_tree, &mut db_txn, &inherent, 2, 0),
        Err(AccountError::InvalidInherent)
    );
}

#[test]
fn reward_inherents_not_allowed() {
    let env = VolatileEnvironment::new(10).unwrap();
    let accounts_tree = AccountsTrie::new(env.clone(), "AccountsTrie");
    let mut db_txn = WriteTransaction::new(&env);

    make_sample_contract(&accounts_tree, &mut db_txn, true);

    let validator_address = Address::from_any_str(VALIDATOR_ADDRESS).unwrap();

    let inherent = Inherent {
        ty: InherentType::Reward,
        target: validator_address,
        value: Coin::ZERO,
        data: vec![],
    };

    assert_eq!(
        StakingContract::commit_inherent(&accounts_tree, &mut db_txn, &inherent, 2, 0),
        Err(AccountError::InvalidForTarget)
    );
}

#[test]
fn slash_inherents_work() {
    let env = VolatileEnvironment::new(10).unwrap();
    let accounts_tree = AccountsTrie::new(env.clone(), "AccountsTrie");
    let mut db_txn = WriteTransaction::new(&env);

    make_sample_contract(&accounts_tree, &mut db_txn, true);

    let validator_address = Address::from_any_str(VALIDATOR_ADDRESS).unwrap();

    // Wrong data.
    let inherent = Inherent {
        ty: InherentType::Slash,
        target: validator_address.clone(),
        value: Coin::ZERO,
        data: vec![],
    };

    assert_eq!(
        StakingContract::commit_inherent(&accounts_tree, &mut db_txn, &inherent, 0, 0),
        Err(AccountError::InvalidInherent)
    );

    // Prepare some data.
    let slot = SlashedSlot {
        slot: 0,
        validator_address: validator_address.clone(),
        event_block: 1,
    };

    let inherent = Inherent {
        ty: InherentType::Slash,
        target: Default::default(),
        value: Coin::ZERO,
        data: slot.serialize_to_vec(),
    };

    // Works in current epoch, current batch case.
    let receipt = SlashReceipt {
        newly_parked: true,
        newly_disabled: true,
        newly_lost_rewards: true,
    }
    .serialize_to_vec();

    assert_eq!(
        StakingContract::commit_inherent(&accounts_tree, &mut db_txn, &inherent, 1, 0),
        Ok(Some(receipt.clone()))
    );

    let staking_contract = StakingContract::get_staking_contract(&accounts_tree, &db_txn);

    assert!(staking_contract.parked_set.contains(&validator_address));
    assert!(staking_contract
        .current_lost_rewards
        .contains(slot.slot as usize));
    assert!(!staking_contract
        .previous_lost_rewards
        .contains(slot.slot as usize));
    assert!(staking_contract
        .current_disabled_slots
        .get(&validator_address)
        .unwrap()
        .contains(&slot.slot));
    assert!(staking_contract
        .previous_disabled_slots
        .get(&validator_address)
        .is_none());

    revert_slash_inherent(
        &accounts_tree,
        &mut db_txn,
        &inherent,
        1,
        Some(&receipt),
        &validator_address,
        slot.slot,
    );

    // Works in current epoch, previous batch case.
    let receipt = SlashReceipt {
        newly_parked: true,
        newly_disabled: true,
        newly_lost_rewards: true,
    }
    .serialize_to_vec();

    assert_eq!(
        StakingContract::commit_inherent(
            &accounts_tree,
            &mut db_txn,
            &inherent,
            1 + BLOCKS_PER_BATCH,
            0
        ),
        Ok(Some(receipt.clone()))
    );

    let staking_contract = StakingContract::get_staking_contract(&accounts_tree, &db_txn);

    assert!(staking_contract.parked_set.contains(&validator_address));
    assert!(!staking_contract
        .current_lost_rewards
        .contains(slot.slot as usize));
    assert!(staking_contract
        .previous_lost_rewards
        .contains(slot.slot as usize));
    assert!(staking_contract
        .current_disabled_slots
        .get(&validator_address)
        .unwrap()
        .contains(&slot.slot));
    assert!(staking_contract
        .previous_disabled_slots
        .get(&validator_address)
        .is_none());

    revert_slash_inherent(
        &accounts_tree,
        &mut db_txn,
        &inherent,
        1 + BLOCKS_PER_BATCH,
        Some(&receipt),
        &validator_address,
        slot.slot,
    );

    // Works in previous epoch, previous batch case.
    let receipt = SlashReceipt {
        newly_parked: true,
        newly_disabled: false,
        newly_lost_rewards: true,
    }
    .serialize_to_vec();

    assert_eq!(
        StakingContract::commit_inherent(
            &accounts_tree,
            &mut db_txn,
            &inherent,
            1 + BLOCKS_PER_EPOCH,
            0
        ),
        Ok(Some(receipt.clone()))
    );

    let staking_contract = StakingContract::get_staking_contract(&accounts_tree, &db_txn);

    assert!(staking_contract.parked_set.contains(&validator_address));
    assert!(!staking_contract
        .current_lost_rewards
        .contains(slot.slot as usize));
    assert!(staking_contract
        .previous_lost_rewards
        .contains(slot.slot as usize));
    assert!(staking_contract
        .current_disabled_slots
        .get(&validator_address)
        .is_none());
    assert!(staking_contract
        .previous_disabled_slots
        .get(&validator_address)
        .is_none());

    revert_slash_inherent(
        &accounts_tree,
        &mut db_txn,
        &inherent,
        1 + BLOCKS_PER_EPOCH,
        Some(&receipt),
        &validator_address,
        slot.slot,
    );
}

#[test]
fn finalize_batch_inherents_work() {
    let env = VolatileEnvironment::new(10).unwrap();
    let accounts_tree = AccountsTrie::new(env.clone(), "AccountsTrie");
    let mut db_txn = WriteTransaction::new(&env);

    make_sample_contract(&accounts_tree, &mut db_txn, true);

    let validator_address = Address::from_any_str(VALIDATOR_ADDRESS).unwrap();

    // Prepare the staking contract.
    let mut staking_contract = StakingContract::get_staking_contract(&accounts_tree, &db_txn);

    staking_contract.current_lost_rewards.insert(0);
    staking_contract.previous_lost_rewards.insert(1);

    accounts_tree.put(
        &mut db_txn,
        &StakingContract::get_key_staking_contract(),
        Account::Staking(staking_contract),
    );

    // Wrong data.
    let inherent = Inherent {
        ty: InherentType::FinalizeBatch,
        target: validator_address,
        value: Coin::ZERO,
        data: vec![123],
    };

    assert_eq!(
        StakingContract::commit_inherent(&accounts_tree, &mut db_txn, &inherent, 0, 0),
        Err(AccountError::InvalidInherent)
    );

    // Works in the valid case.
    let inherent = Inherent {
        ty: InherentType::FinalizeBatch,
        target: Default::default(),
        value: Coin::ZERO,
        data: vec![],
    };

    assert_eq!(
        StakingContract::commit_inherent(&accounts_tree, &mut db_txn, &inherent, 1, 0),
        Ok(None)
    );

    let staking_contract = StakingContract::get_staking_contract(&accounts_tree, &db_txn);

    assert!(staking_contract.parked_set.is_empty());
    assert!(staking_contract.current_lost_rewards.is_empty());
    assert!(staking_contract.previous_lost_rewards.contains(0));
    assert!(staking_contract.current_disabled_slots.is_empty());
    assert!(staking_contract.previous_disabled_slots.is_empty());

    // Cannot revert the inherent.
    assert_eq!(
        StakingContract::revert_inherent(&accounts_tree, &mut db_txn, &inherent, 1, 0, None),
        Err(AccountError::InvalidForTarget)
    );
}

#[test]
fn finalize_epoch_inherents_work() {
    let env = VolatileEnvironment::new(10).unwrap();
    let accounts_tree = AccountsTrie::new(env.clone(), "AccountsTrie");
    let mut db_txn = WriteTransaction::new(&env);

    make_sample_contract(&accounts_tree, &mut db_txn, true);

    let validator_address = Address::from_any_str(VALIDATOR_ADDRESS).unwrap();

    // Prepare the staking contract.
    let mut staking_contract = StakingContract::get_staking_contract(&accounts_tree, &db_txn);

    staking_contract
        .parked_set
        .insert(validator_address.clone());
    staking_contract.current_lost_rewards.insert(0);
    staking_contract.previous_lost_rewards.insert(1);
    let mut set_c = BTreeSet::new();
    set_c.insert(0);
    staking_contract
        .current_disabled_slots
        .insert(validator_address.clone(), set_c.clone());
    let mut set_p = BTreeSet::new();
    set_p.insert(1);
    staking_contract
        .previous_disabled_slots
        .insert(validator_address.clone(), set_p);

    accounts_tree.put(
        &mut db_txn,
        &StakingContract::get_key_staking_contract(),
        Account::Staking(staking_contract),
    );

    // Wrong data.
    let inherent = Inherent {
        ty: InherentType::FinalizeEpoch,
        target: validator_address.clone(),
        value: Coin::ZERO,
        data: vec![123],
    };

    assert_eq!(
        StakingContract::commit_inherent(&accounts_tree, &mut db_txn, &inherent, 0, 0),
        Err(AccountError::InvalidInherent)
    );

    // Works in the valid case.
    let inherent = Inherent {
        ty: InherentType::FinalizeEpoch,
        target: Default::default(),
        value: Coin::ZERO,
        data: vec![],
    };

    assert_eq!(
        StakingContract::commit_inherent(&accounts_tree, &mut db_txn, &inherent, 1, 0),
        Ok(None)
    );

    let staking_contract = StakingContract::get_staking_contract(&accounts_tree, &db_txn);

    assert!(!staking_contract
        .active_validators
        .contains_key(&validator_address));

    assert!(staking_contract.parked_set.is_empty());

    assert!(staking_contract.current_lost_rewards.is_empty());
    let mut bitset = BitSet::new();
    bitset.insert(0);
    assert_eq!(staking_contract.previous_lost_rewards, bitset);

    assert!(staking_contract.current_disabled_slots.is_empty());
    assert_eq!(
        staking_contract
            .previous_disabled_slots
            .get(&validator_address)
            .unwrap(),
        &set_c
    );

    let validator =
        StakingContract::get_validator(&accounts_tree, &db_txn, &validator_address).unwrap();

    assert_eq!(validator.inactivity_flag, Some(1));

    // Cannot revert the inherent.
    assert_eq!(
        StakingContract::revert_inherent(&accounts_tree, &mut db_txn, &inherent, 1, 0, None),
        Err(AccountError::InvalidForTarget)
    );
}

fn make_empty_contract(accounts_tree: &AccountsTrie, db_txn: &mut WriteTransaction) {
    StakingContract::create(accounts_tree, db_txn)
}

fn make_sample_contract(
    accounts_tree: &AccountsTrie,
    db_txn: &mut WriteTransaction,
    with_staker: bool,
) {
    let staker_address = Address::from_any_str(STAKER_ADDRESS).unwrap();

    let cold_address = Address::from_any_str(VALIDATOR_ADDRESS).unwrap();

    let signing_key =
        PublicKey::deserialize_from_vec(&hex::decode(VALIDATOR_SIGNING_KEY).unwrap()).unwrap();

    let voting_key =
        BlsPublicKey::deserialize_from_vec(&hex::decode(VALIDATOR_VOTING_KEY).unwrap()).unwrap();

    make_empty_contract(accounts_tree, db_txn);

    StakingContract::create_validator(
        accounts_tree,
        db_txn,
        &cold_address,
        signing_key,
        voting_key,
        cold_address.clone(),
        None,
    )
    .unwrap();

    if with_staker {
        StakingContract::create_staker(
            accounts_tree,
            db_txn,
            &staker_address,
            Coin::from_u64_unchecked(150_000_000),
            Some(cold_address),
        )
        .unwrap();
    }
}

fn make_incoming_transaction(data: IncomingStakingTransactionData, value: u64) -> Transaction {
    match data {
        IncomingStakingTransactionData::CreateValidator { .. }
        | IncomingStakingTransactionData::CreateStaker { .. }
        | IncomingStakingTransactionData::Stake { .. } => Transaction::new_extended(
            Address::from_any_str(STAKER_ADDRESS).unwrap(),
            AccountType::Basic,
            STAKING_CONTRACT_ADDRESS,
            AccountType::Staking,
            value.try_into().unwrap(),
            100.try_into().unwrap(),
            data.serialize_to_vec(),
            1,
            NetworkId::Dummy,
        ),
        _ => Transaction::new_signalling(
            Address::from_any_str(STAKER_ADDRESS).unwrap(),
            AccountType::Basic,
            STAKING_CONTRACT_ADDRESS,
            AccountType::Staking,
            value.try_into().unwrap(),
            100.try_into().unwrap(),
            data.serialize_to_vec(),
            1,
            NetworkId::Dummy,
        ),
    }
}

fn make_signed_incoming_transaction(
    data: IncomingStakingTransactionData,
    value: u64,
    in_key_pair: &KeyPair,
) -> Transaction {
    let mut tx = make_incoming_transaction(data, value);

    let in_proof = SignatureProof::from(
        in_key_pair.public,
        in_key_pair.sign(&tx.serialize_content()),
    );

    tx.data = IncomingStakingTransactionData::set_signature_on_data(&tx.data, in_proof).unwrap();

    let out_private_key =
        PrivateKey::deserialize_from_vec(&hex::decode(STAKER_PRIVATE_KEY).unwrap()).unwrap();

    let out_key_pair = KeyPair::from(out_private_key);

    let out_proof = SignatureProof::from(
        out_key_pair.public,
        out_key_pair.sign(&tx.serialize_content()),
    )
    .serialize_to_vec();

    tx.proof = out_proof;

    tx
}

fn make_delete_validator_transaction() -> Transaction {
    let mut tx = Transaction::new_extended(
        STAKING_CONTRACT_ADDRESS,
        AccountType::Staking,
        Address::from_any_str(STAKER_ADDRESS).unwrap(),
        AccountType::Basic,
        (VALIDATOR_DEPOSIT - 100).try_into().unwrap(),
        100.try_into().unwrap(),
        vec![],
        1,
        NetworkId::Dummy,
    );

    let private_key =
        PrivateKey::deserialize_from_vec(&hex::decode(VALIDATOR_PRIVATE_KEY).unwrap()).unwrap();

    let key_pair = KeyPair::from(private_key);

    let sig = SignatureProof::from(key_pair.public, key_pair.sign(&tx.serialize_content()));

    let proof = OutgoingStakingTransactionProof::DeleteValidator { proof: sig };

    tx.proof = proof.serialize_to_vec();

    tx
}

fn make_unstake_transaction(value: u64) -> Transaction {
    let mut tx = Transaction::new_extended(
        STAKING_CONTRACT_ADDRESS,
        AccountType::Staking,
        Address::from_any_str(STAKER_ADDRESS).unwrap(),
        AccountType::Basic,
        (value - 100).try_into().unwrap(),
        100.try_into().unwrap(),
        vec![],
        1,
        NetworkId::Dummy,
    );

    let private_key =
        PrivateKey::deserialize_from_vec(&hex::decode(STAKER_PRIVATE_KEY).unwrap()).unwrap();

    let key_pair = KeyPair::from(private_key);

    let sig = SignatureProof::from(key_pair.public, key_pair.sign(&tx.serialize_content()));

    let proof = OutgoingStakingTransactionProof::Unstake { proof: sig };

    tx.proof = proof.serialize_to_vec();

    tx
}

fn revert_slash_inherent(
    accounts_tree: &AccountsTrie,
    db_txn: &mut WriteTransaction,
    inherent: &Inherent,
    block_height: u32,
    receipt: Option<&Vec<u8>>,
    validator_address: &Address,
    slot: u16,
) {
    assert_eq!(
        StakingContract::revert_inherent(accounts_tree, db_txn, inherent, block_height, 0, receipt),
        Ok(())
    );

    let staking_contract = StakingContract::get_staking_contract(accounts_tree, db_txn);

    assert!(!staking_contract.parked_set.contains(validator_address));
    assert!(!staking_contract
        .current_lost_rewards
        .contains(slot as usize));
    assert!(!staking_contract
        .previous_lost_rewards
        .contains(slot as usize));
    assert!(staking_contract
        .current_disabled_slots
        .get(validator_address)
        .is_none());
    assert!(staking_contract
        .previous_disabled_slots
        .get(validator_address)
        .is_none());
}

fn bls_key_pair(sk: &str) -> BlsKeyPair {
    BlsKeyPair::from(BlsSecretKey::deserialize_from_vec(&hex::decode(sk).unwrap()).unwrap())
}

fn ed25519_key_pair(sk: &str) -> KeyPair {
    KeyPair::from(PrivateKey::deserialize_from_vec(&hex::decode(sk).unwrap()).unwrap())
}
