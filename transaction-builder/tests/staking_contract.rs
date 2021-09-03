use std::convert::TryInto;

use beserial::{Deserialize, Serialize};
use nimiq_bls::KeyPair as BlsKeyPair;
use nimiq_hash::Blake2bHash;
use nimiq_keys::{Address, KeyPair, PrivateKey};
use nimiq_primitives::account::AccountType;
use nimiq_primitives::coin::Coin;
use nimiq_primitives::networks::NetworkId;
use nimiq_primitives::policy::{STAKING_CONTRACT_ADDRESS, VALIDATOR_DEPOSIT};
use nimiq_transaction::account::staking_contract::{
    IncomingStakingTransactionData, OutgoingStakingTransactionProof,
};
use nimiq_transaction::{SignatureProof, Transaction};
use nimiq_transaction_builder::TransactionBuilder;

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
    );

    assert_eq!(tx, tx2);

    // Stake
    let tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::Stake {
            staker_address: address.clone(),
        },
        100_000_000,
        &key_pair,
    );

    let tx2 = TransactionBuilder::new_stake(
        &key_pair,
        address,
        100_000_000.try_into().unwrap(),
        100.try_into().unwrap(),
        1,
        NetworkId::Dummy,
    );

    assert_eq!(tx, tx2);

    // Update
    let tx = make_self_transaction(
        IncomingStakingTransactionData::UpdateStaker {
            new_delegation: None,
            proof: Default::default(),
        },
        &key_pair,
        true,
    );

    let tx2 = TransactionBuilder::new_update_staker(
        &key_pair,
        None,
        true,
        100.try_into().unwrap(),
        1,
        NetworkId::Dummy,
    );

    assert_eq!(tx, tx2);

    // Retire
    let tx = make_self_transaction(
        IncomingStakingTransactionData::RetireStaker {
            value: Coin::from_u64_unchecked(1000),
            proof: Default::default(),
        },
        &key_pair,
        true,
    );

    let tx2 = TransactionBuilder::new_retire_staker(
        &key_pair,
        true,
        1000.try_into().unwrap(),
        100.try_into().unwrap(),
        1,
        NetworkId::Dummy,
    );

    assert_eq!(tx, tx2);

    // Reactivate
    let tx = make_self_transaction(
        IncomingStakingTransactionData::ReactivateStaker {
            value: Coin::from_u64_unchecked(1000),
            proof: Default::default(),
        },
        &key_pair,
        true,
    );

    let tx2 = TransactionBuilder::new_reactivate_staker(
        &key_pair,
        true,
        1000.try_into().unwrap(),
        100.try_into().unwrap(),
        1,
        NetworkId::Dummy,
    );

    assert_eq!(tx, tx2);

    // Unstake
    let tx = make_unstake_transaction(&key_pair, 150_000_000);

    let tx2 = TransactionBuilder::new_unstake(
        &key_pair,
        Address::from_any_str(ADDRESS).unwrap(),
        150_000_000.try_into().unwrap(),
        100.try_into().unwrap(),
        1,
        NetworkId::Dummy,
    );

    assert_eq!(tx, tx2);
}

#[test]
fn it_can_create_validator_transactions() {
    let bls_pair = bls_key_pair();
    let key_pair = ed25519_key_pair();
    let address = Address::from_any_str(ADDRESS).unwrap();

    // Create validator
    let tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::CreateValidator {
            warm_key: address.clone(),
            validator_key: bls_pair.public_key.compress(),
            proof_of_knowledge: bls_pair.sign(&bls_pair.public_key).compress(),
            reward_address: address.clone(),
            signal_data: Some(Blake2bHash::default()),
            proof: Default::default(),
        },
        VALIDATOR_DEPOSIT,
        &key_pair,
    );

    let tx2 = TransactionBuilder::new_create_validator(
        &key_pair,
        &key_pair,
        address.clone(),
        &bls_pair,
        address.clone(),
        Some(Blake2bHash::default()),
        100.try_into().unwrap(),
        1,
        NetworkId::Dummy,
    );

    assert_eq!(tx, tx2);

    // Update
    let tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::UpdateValidator {
            new_warm_key: Some(address.clone()),
            new_validator_key: None,
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
        Some(address.clone()),
        None,
        Some(address.clone()),
        None,
        100.try_into().unwrap(),
        1,
        NetworkId::Dummy,
    );

    assert_eq!(tx, tx2);

    // Retire
    let tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::RetireValidator {
            validator_address: address.clone(),
            proof: Default::default(),
        },
        0,
        &key_pair,
    );

    let tx2 = TransactionBuilder::new_retire_validator(
        &key_pair,
        address.clone(),
        &key_pair,
        100.try_into().unwrap(),
        1,
        NetworkId::Dummy,
    );

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
    );

    assert_eq!(tx, tx2);

    // Unpark
    let tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::UnparkValidator {
            validator_address: address.clone(),
            proof: Default::default(),
        },
        0,
        &key_pair,
    );

    let tx2 = TransactionBuilder::new_unpark_validator(
        &key_pair,
        address.clone(),
        &key_pair,
        100.try_into().unwrap(),
        1,
        NetworkId::Dummy,
    );

    assert_eq!(tx, tx2);

    // Drop
    let tx = make_drop_transaction(&key_pair, VALIDATOR_DEPOSIT - 100);

    let tx2 = TransactionBuilder::new_drop_validator(
        address,
        &key_pair,
        100.try_into().unwrap(),
        1,
        NetworkId::Dummy,
    );

    assert_eq!(tx, tx2);
}

fn make_incoming_transaction(data: IncomingStakingTransactionData, value: u64) -> Transaction {
    match data {
        IncomingStakingTransactionData::CreateStaker { .. }
        | IncomingStakingTransactionData::Stake { .. }
        | IncomingStakingTransactionData::CreateValidator { .. } => Transaction::new_extended(
            Address::from_any_str(ADDRESS).unwrap(),
            AccountType::Basic,
            Address::from_any_str(STAKING_CONTRACT_ADDRESS).unwrap(),
            AccountType::Staking,
            value.try_into().unwrap(),
            100.try_into().unwrap(),
            data.serialize_to_vec(),
            1,
            NetworkId::Dummy,
        ),
        _ => Transaction::new_signalling(
            Address::from_any_str(ADDRESS).unwrap(),
            AccountType::Basic,
            Address::from_any_str(STAKING_CONTRACT_ADDRESS).unwrap(),
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
    key_pair: &KeyPair,
) -> Transaction {
    let mut tx = make_incoming_transaction(data, value);
    tx.data = IncomingStakingTransactionData::set_signature_on_data(
        &tx.data,
        SignatureProof::from(key_pair.public, key_pair.sign(&tx.serialize_content())),
    )
    .unwrap();

    tx.proof = SignatureProof::from(key_pair.public, key_pair.sign(&tx.serialize_content()))
        .serialize_to_vec();
    tx
}

fn make_unstake_transaction(key_pair: &KeyPair, value: u64) -> Transaction {
    let mut tx = Transaction::new_extended(
        Address::from_any_str(STAKING_CONTRACT_ADDRESS).unwrap(),
        AccountType::Staking,
        Address::from_any_str(ADDRESS).unwrap(),
        AccountType::Basic,
        value.try_into().unwrap(),
        100.try_into().unwrap(),
        vec![],
        1,
        NetworkId::Dummy,
    );
    let proof = OutgoingStakingTransactionProof::Unstake {
        proof: SignatureProof::from(key_pair.public, key_pair.sign(&tx.serialize_content())),
    };
    tx.proof = proof.serialize_to_vec();
    tx
}

fn make_drop_transaction(key_pair: &KeyPair, value: u64) -> Transaction {
    let mut tx = Transaction::new_extended(
        Address::from_any_str(STAKING_CONTRACT_ADDRESS).unwrap(),
        AccountType::Staking,
        Address::from_any_str(ADDRESS).unwrap(),
        AccountType::Basic,
        value.try_into().unwrap(),
        100.try_into().unwrap(),
        vec![],
        1,
        NetworkId::Dummy,
    );
    let proof = OutgoingStakingTransactionProof::DropValidator {
        proof: SignatureProof::from(key_pair.public, key_pair.sign(&tx.serialize_content())),
    };
    tx.proof = proof.serialize_to_vec();
    tx
}

fn make_self_transaction(
    data: IncomingStakingTransactionData,
    key_pair: &KeyPair,
    from_active_balance: bool,
) -> Transaction {
    let mut tx = Transaction::new_signalling(
        Address::from_any_str(STAKING_CONTRACT_ADDRESS).unwrap(),
        AccountType::Staking,
        Address::from_any_str(STAKING_CONTRACT_ADDRESS).unwrap(),
        AccountType::Staking,
        0.try_into().unwrap(),
        100.try_into().unwrap(),
        data.serialize_to_vec(),
        1,
        NetworkId::Dummy,
    );
    tx.data = IncomingStakingTransactionData::set_signature_on_data(
        &tx.data,
        SignatureProof::from(key_pair.public, key_pair.sign(&tx.serialize_content())),
    )
    .unwrap();
    let proof = OutgoingStakingTransactionProof::DeductFees {
        from_active_balance,
        proof: SignatureProof::from(key_pair.public, key_pair.sign(&tx.serialize_content())),
    };
    tx.proof = proof.serialize_to_vec();
    tx
}

fn bls_key_pair() -> BlsKeyPair {
    BlsKeyPair::from_secret(
        &Deserialize::deserialize(&mut &hex::decode(BLS_PRIVKEY).unwrap()[..]).unwrap(),
    )
}

fn ed25519_key_pair() -> KeyPair {
    let priv_key: PrivateKey =
        Deserialize::deserialize(&mut &hex::decode(PRIVATE_KEY).unwrap()[..]).unwrap();
    priv_key.into()
}
