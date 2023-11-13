use std::convert::TryInto;

use nimiq_bls::KeyPair as BlsKeyPair;
use nimiq_hash::Blake2bHash;
use nimiq_keys::{Address, KeyPair, PrivateKey};
use nimiq_primitives::{account::AccountType, coin::Coin, networks::NetworkId, policy::Policy};
use nimiq_serde::{Deserialize, Serialize};
use nimiq_test_log::test;
use nimiq_transaction::{
    account::staking_contract::{IncomingStakingTransactionData, OutgoingStakingTransactionData},
    SignatureProof, Transaction,
};
use nimiq_transaction_builder::{TransactionBuilder, TransactionBuilderError};

const ADDRESS: &str = "9cd82948650d902d95d52ea2ec91eae6deb0c9fe";
const PRIVATE_KEY: &str = "b410a7a583cbc13ef4f1cbddace30928bcb4f9c13722414bc4a2faaba3f4e187";
const BLS_PRIVKEY: &str = "93ded88af373537a2fad738892ae29cf012bb27875cb66af9278991acbcb8e44f414\
    9c27fe9d62a31ae8537fc4891e935e1303c511091095c0ad083a1cfc0f5f223c394c2d5109288e639cde0692facc9fd\
    221a806c0003835db99b423360000";

#[test]
fn it_can_create_staker_transactions() {
    let key_pair = ed25519_key_pair();
    let address = Address::from_any_str(ADDRESS).unwrap();

    // Create staker
    let tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::CreateStaker {
            delegation: Some(address.clone()),
            proof: Default::default(),
        },
        100_000_000,
        &key_pair,
    );

    let tx2 = TransactionBuilder::new_create_staker(
        &key_pair,
        &key_pair,
        Some(address.clone()),
        100_000_000.try_into().unwrap(),
        100.try_into().unwrap(),
        1,
        NetworkId::Dummy,
    )
    .unwrap();

    assert_eq!(tx, tx2);

    // Stake
    let tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::AddStake {
            staker_address: address.clone(),
        },
        100_000_000,
        &key_pair,
    );

    let tx2 = TransactionBuilder::new_add_stake(
        &key_pair,
        address,
        100_000_000.try_into().unwrap(),
        100.try_into().unwrap(),
        1,
        NetworkId::Dummy,
    )
    .unwrap();

    assert_eq!(tx, tx2);

    // Update (fees from basic account)
    let tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::UpdateStaker {
            new_delegation: None,
            reactivate_all_stake: false,
            proof: Default::default(),
        },
        0,
        &key_pair,
    );

    let tx2 = TransactionBuilder::new_update_staker(
        Some(&key_pair),
        &key_pair,
        None,
        false,
        100.try_into().unwrap(),
        1,
        NetworkId::Dummy,
    )
    .unwrap();

    assert_eq!(tx, tx2);

    let tx2 = TransactionBuilder::new_update_staker(
        None,
        &key_pair,
        None,
        false,
        100.try_into().unwrap(),
        1,
        NetworkId::Dummy,
    )
    .unwrap();

    assert_eq!(tx, tx2);

    // Remove stake.
    let tx = make_remove_stake_transaction(&key_pair, 150_000_000);

    let tx2 = TransactionBuilder::new_remove_stake(
        &key_pair,
        Address::from_any_str(ADDRESS).unwrap(),
        150_000_000.try_into().unwrap(),
        100.try_into().unwrap(),
        1,
        NetworkId::Dummy,
    )
    .unwrap();

    assert_eq!(tx, tx2);
}

#[test]
fn it_can_fail_creating_staker_transactions() {
    let key_pair = ed25519_key_pair();
    let address = Address::from_any_str(ADDRESS).unwrap();

    // Invalid stake
    let err3 = TransactionBuilder::new_add_stake(
        &key_pair,
        address,
        0.try_into().unwrap(), // InvalidValue
        100.try_into().unwrap(),
        1,
        NetworkId::Dummy,
    )
    .err();

    match err3 {
        Some(TransactionBuilderError::InvalidValue) => {}
        Some(_) => panic!("Wrong error returned"),
        None => panic!("No error returned"),
    }
}

#[test]
fn it_can_create_validator_transactions() {
    let bls_pair = bls_key_pair();
    let key_pair = ed25519_key_pair();
    let address = Address::from_any_str(ADDRESS).unwrap();

    // Create validator
    let tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::CreateValidator {
            signing_key: key_pair.public,
            voting_key: bls_pair.public_key.compress(),
            proof_of_knowledge: bls_pair.sign(&bls_pair.public_key).compress(),
            reward_address: address.clone(),
            signal_data: Some(Blake2bHash::default()),
            proof: Default::default(),
        },
        Policy::VALIDATOR_DEPOSIT,
        &key_pair,
    );

    let tx2 = TransactionBuilder::new_create_validator(
        &key_pair,
        &key_pair,
        key_pair.public,
        &bls_pair,
        address.clone(),
        Some(Blake2bHash::default()),
        100.try_into().unwrap(),
        1,
        NetworkId::Dummy,
    )
    .unwrap();

    assert_eq!(tx, tx2);

    // Update
    let tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::UpdateValidator {
            new_signing_key: Some(key_pair.public),
            new_voting_key: None,
            new_proof_of_knowledge: None,
            new_reward_address: Some(address.clone()),
            new_signal_data: None,
            proof: Default::default(),
        },
        0,
        &key_pair,
    );

    let tx2 = TransactionBuilder::new_update_validator(
        &key_pair,
        &key_pair,
        Some(key_pair.public),
        None,
        Some(address.clone()),
        None,
        100.try_into().unwrap(),
        1,
        NetworkId::Dummy,
    )
    .unwrap();

    assert_eq!(tx, tx2);

    // Deactivate
    let tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::DeactivateValidator {
            validator_address: address.clone(),
            proof: Default::default(),
        },
        0,
        &key_pair,
    );

    let tx2 = TransactionBuilder::new_deactivate_validator(
        &key_pair,
        address.clone(),
        &key_pair,
        100.try_into().unwrap(),
        1,
        NetworkId::Dummy,
    )
    .unwrap();

    assert_eq!(tx, tx2);

    // Reactivate
    let tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::ReactivateValidator {
            validator_address: address.clone(),
            proof: Default::default(),
        },
        0,
        &key_pair,
    );

    let tx2 = TransactionBuilder::new_reactivate_validator(
        &key_pair,
        address.clone(),
        &key_pair,
        100.try_into().unwrap(),
        1,
        NetworkId::Dummy,
    )
    .unwrap();

    assert_eq!(tx, tx2);

    // Delete
    let tx = make_delete_transaction(&key_pair, Policy::VALIDATOR_DEPOSIT - 100);

    let tx2 = TransactionBuilder::new_delete_validator(
        address,
        &key_pair,
        100.try_into().unwrap(),
        Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT - 100),
        1,
        NetworkId::Dummy,
    )
    .unwrap();

    assert_eq!(tx, tx2);
}

fn make_incoming_transaction(data: IncomingStakingTransactionData, value: u64) -> Transaction {
    match data {
        IncomingStakingTransactionData::CreateStaker { .. }
        | IncomingStakingTransactionData::AddStake { .. }
        | IncomingStakingTransactionData::CreateValidator { .. } => Transaction::new_extended(
            Address::from_any_str(ADDRESS).unwrap(),
            AccountType::Basic,
            vec![],
            Policy::STAKING_CONTRACT_ADDRESS,
            AccountType::Staking,
            data.serialize_to_vec(),
            value.try_into().unwrap(),
            100.try_into().unwrap(),
            1,
            NetworkId::Dummy,
        ),
        _ => Transaction::new_signaling(
            Address::from_any_str(ADDRESS).unwrap(),
            AccountType::Basic,
            Policy::STAKING_CONTRACT_ADDRESS,
            AccountType::Staking,
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
    key_pair: &KeyPair,
) -> Transaction {
    let mut tx = make_incoming_transaction(data, value);
    tx.recipient_data = IncomingStakingTransactionData::set_signature_on_data(
        &tx.recipient_data,
        SignatureProof::from_ed25519(key_pair.public, key_pair.sign(&tx.serialize_content())),
    )
    .unwrap();

    tx.proof =
        SignatureProof::from_ed25519(key_pair.public, key_pair.sign(&tx.serialize_content()))
            .serialize_to_vec();
    tx
}

fn make_remove_stake_transaction(key_pair: &KeyPair, value: u64) -> Transaction {
    let mut tx = Transaction::new_extended(
        Policy::STAKING_CONTRACT_ADDRESS,
        AccountType::Staking,
        OutgoingStakingTransactionData::RemoveStake.serialize_to_vec(),
        Address::from_any_str(ADDRESS).unwrap(),
        AccountType::Basic,
        vec![],
        value.try_into().unwrap(),
        100.try_into().unwrap(),
        1,
        NetworkId::Dummy,
    );
    tx.proof = key_pair.sign(&tx.serialize_content()).serialize_to_vec();
    tx
}

//TODO update this function with a more proper interface
fn make_delete_transaction(key_pair: &KeyPair, value: u64) -> Transaction {
    let mut tx = Transaction::new_extended(
        Policy::STAKING_CONTRACT_ADDRESS,
        AccountType::Staking,
        OutgoingStakingTransactionData::DeleteValidator.serialize_to_vec(),
        Address::from_any_str(ADDRESS).unwrap(),
        AccountType::Basic,
        vec![],
        value.try_into().unwrap(),
        100.try_into().unwrap(),
        1,
        NetworkId::Dummy,
    );
    tx.proof = key_pair.sign(&tx.serialize_content()).serialize_to_vec();
    tx
}

fn bls_key_pair() -> BlsKeyPair {
    BlsKeyPair::from_secret(
        &Deserialize::deserialize_from_vec(&hex::decode(BLS_PRIVKEY).unwrap()[..]).unwrap(),
    )
}

fn ed25519_key_pair() -> KeyPair {
    let priv_key: PrivateKey =
        Deserialize::deserialize_from_vec(&hex::decode(PRIVATE_KEY).unwrap()[..]).unwrap();
    priv_key.into()
}
