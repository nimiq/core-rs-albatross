use std::convert::TryInto;

use beserial::{Deserialize, Serialize};
use nimiq_account::AccountType;
use nimiq_bls::KeyPair as BlsKeyPair;
use nimiq_keys::{Address, KeyPair, PrivateKey};
use nimiq_primitives::networks::NetworkId;
use nimiq_transaction::account::staking_contract::{
    IncomingStakingTransactionData, OutgoingStakingTransactionProof, SelfStakingTransactionData,
};
use nimiq_transaction::{SignatureProof, Transaction};
use nimiq_transaction_builder::{Recipient, TransactionBuilder};

const STAKER_ADDRESS: &str = "9cd82948650d902d95d52ea2ec91eae6deb0c9fe";
const STAKER_PRIVATE_KEY: &str = "b410a7a583cbc13ef4f1cbddace30928bcb4f9c13722414bc4a2faaba3f4e187";

#[test]
fn it_can_verify_staker_transactions() {
    let bls_pair = bls_key_pair();
    let key_pair = ed25519_key_pair();

    // Staking
    let tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::Stake {
            validator_key: bls_pair.public_key.compress(),
            staker_address: None,
        },
        150_000_000,
        &bls_pair,
        &key_pair,
    );

    let tx2 = TransactionBuilder::new_stake(
        Address::from([1u8; 20]),
        &key_pair,
        &bls_pair.public_key,
        150_000_000.try_into().unwrap(),
        100.try_into().unwrap(),
        1,
        NetworkId::Dummy,
    );

    assert_eq!(tx2, tx);

    // Retire
    let tx = make_self_transaction(
        SelfStakingTransactionData::RetireStake(bls_pair.public_key.compress()),
        150_000_000,
    );

    let tx2 = TransactionBuilder::new_retire(
        Address::from([1u8; 20]),
        &key_pair,
        &bls_pair.public_key,
        150_000_000.try_into().unwrap(),
        100.try_into().unwrap(),
        1,
        NetworkId::Dummy,
    );

    assert_eq!(tx2, tx);

    // Reactivate
    let tx = make_self_transaction(
        SelfStakingTransactionData::ReactivateStake(bls_pair.public_key.compress()),
        150_000_000,
    );

    let tx2 = TransactionBuilder::new_reactivate(
        Address::from([1u8; 20]),
        &key_pair,
        &bls_pair.public_key,
        150_000_000.try_into().unwrap(),
        100.try_into().unwrap(),
        1,
        NetworkId::Dummy,
    );

    assert_eq!(tx2, tx);

    // Unstake
    let tx = make_unstake_transaction(&key_pair, 150_000_000);

    let tx2 = TransactionBuilder::new_unstake(
        Address::from([1u8; 20]),
        &key_pair,
        Address::from_any_str(STAKER_ADDRESS).unwrap(),
        150_000_000.try_into().unwrap(),
        100.try_into().unwrap(),
        1,
        NetworkId::Dummy,
    );

    assert_eq!(tx2, tx);
}

#[test]
fn it_can_verify_validator_transactions() {
    let bls_pair = bls_key_pair();
    let key_pair = ed25519_key_pair();

    // Create
    let tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::CreateValidator {
            validator_key: bls_pair.public_key.compress(),
            proof_of_knowledge: bls_pair.sign(&bls_pair.public_key).compress(),
            reward_address: Address::from_any_str(STAKER_ADDRESS).unwrap(),
        },
        150_000_000,
        &bls_pair,
        &key_pair,
    );

    let mut recipient = Recipient::new_staking_builder(Address::from([1u8; 20]));
    recipient.create_validator(&bls_pair, Address::from_any_str(STAKER_ADDRESS).unwrap());

    let mut tx_builder = TransactionBuilder::new();
    tx_builder
        .with_sender(Address::from_any_str(STAKER_ADDRESS).unwrap())
        .with_value(150_000_000.try_into().unwrap())
        .with_fee(100.try_into().unwrap())
        .with_network_id(NetworkId::Dummy)
        .with_validity_start_height(1)
        .with_recipient(recipient.generate().unwrap());
    let mut proof_builder = tx_builder.generate().unwrap().unwrap_basic();
    proof_builder.sign_with_key_pair(&key_pair);
    let tx2 = proof_builder.generate().unwrap();

    assert_eq!(tx2, tx);

    // Update
    let tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::UpdateValidator {
            old_validator_key: bls_pair.public_key.compress(),
            new_validator_key: None,
            new_proof_of_knowledge: None,
            new_reward_address: Some(Address::from([1u8; 20])),
            signature: Default::default(),
        },
        0,
        &bls_pair,
        &key_pair,
    );

    let mut recipient = Recipient::new_staking_builder(Address::from([1u8; 20]));
    recipient.update_validator(&bls_pair.public_key, None, Some(Address::from([1u8; 20])));

    let mut tx_builder = TransactionBuilder::new();
    tx_builder
        .with_sender(Address::from_any_str(STAKER_ADDRESS).unwrap())
        .with_value(0.try_into().unwrap())
        .with_fee(100.try_into().unwrap())
        .with_network_id(NetworkId::Dummy)
        .with_validity_start_height(1)
        .with_recipient(recipient.generate().unwrap());
    let mut proof_builder = tx_builder.generate().unwrap().unwrap_signalling();
    proof_builder.sign_with_validator_key_pair(&bls_pair);
    let mut proof_builder = proof_builder.generate().unwrap().unwrap_basic();
    proof_builder.sign_with_key_pair(&key_pair);
    let tx2 = proof_builder.generate().unwrap();

    assert_eq!(tx2, tx);

    // Retire
    let tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::RetireValidator {
            validator_key: bls_pair.public_key.compress(),
            signature: Default::default(),
        },
        0,
        &bls_pair,
        &key_pair,
    );

    let mut recipient = Recipient::new_staking_builder(Address::from([1u8; 20]));
    recipient.retire_validator(&bls_pair.public_key);

    let mut tx_builder = TransactionBuilder::new();
    tx_builder
        .with_sender(Address::from_any_str(STAKER_ADDRESS).unwrap())
        .with_value(0.try_into().unwrap())
        .with_fee(100.try_into().unwrap())
        .with_network_id(NetworkId::Dummy)
        .with_validity_start_height(1)
        .with_recipient(recipient.generate().unwrap());
    let mut proof_builder = tx_builder.generate().unwrap().unwrap_signalling();
    proof_builder.sign_with_validator_key_pair(&bls_pair);
    let mut proof_builder = proof_builder.generate().unwrap().unwrap_basic();
    proof_builder.sign_with_key_pair(&key_pair);
    let tx2 = proof_builder.generate().unwrap();

    assert_eq!(tx2, tx);

    // Reactivate
    let tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::ReactivateValidator {
            validator_key: bls_pair.public_key.compress(),
            signature: Default::default(),
        },
        0,
        &bls_pair,
        &key_pair,
    );

    let mut recipient = Recipient::new_staking_builder(Address::from([1u8; 20]));
    recipient.reactivate_validator(&bls_pair.public_key);

    let mut tx_builder = TransactionBuilder::new();
    tx_builder
        .with_sender(Address::from_any_str(STAKER_ADDRESS).unwrap())
        .with_value(0.try_into().unwrap())
        .with_fee(100.try_into().unwrap())
        .with_network_id(NetworkId::Dummy)
        .with_validity_start_height(1)
        .with_recipient(recipient.generate().unwrap());
    let mut proof_builder = tx_builder.generate().unwrap().unwrap_signalling();
    proof_builder.sign_with_validator_key_pair(&bls_pair);
    let mut proof_builder = proof_builder.generate().unwrap().unwrap_basic();
    proof_builder.sign_with_key_pair(&key_pair);
    let tx2 = proof_builder.generate().unwrap();

    assert_eq!(tx2, tx);

    // Unpark
    let tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::UnparkValidator {
            validator_key: bls_pair.public_key.compress(),
            signature: Default::default(),
        },
        0,
        &bls_pair,
        &key_pair,
    );

    let mut recipient = Recipient::new_staking_builder(Address::from([1u8; 20]));
    recipient.unpark_validator(&bls_pair.public_key);

    let mut tx_builder = TransactionBuilder::new();
    tx_builder
        .with_sender(Address::from_any_str(STAKER_ADDRESS).unwrap())
        .with_value(0.try_into().unwrap())
        .with_fee(100.try_into().unwrap())
        .with_network_id(NetworkId::Dummy)
        .with_validity_start_height(1)
        .with_recipient(recipient.generate().unwrap());
    let mut proof_builder = tx_builder.generate().unwrap().unwrap_signalling();
    proof_builder.sign_with_validator_key_pair(&bls_pair);
    let mut proof_builder = proof_builder.generate().unwrap().unwrap_basic();
    proof_builder.sign_with_key_pair(&key_pair);
    let tx2 = proof_builder.generate().unwrap();

    assert_eq!(tx2, tx);

    // Drop
    let tx = make_drop_transaction(&bls_pair, 150_000_000);

    let recipient = Recipient::new_basic(Address::from_any_str(STAKER_ADDRESS).unwrap());

    let mut tx_builder = TransactionBuilder::new();
    tx_builder
        .with_sender(Address::from([1u8; 20]))
        .with_sender_type(AccountType::Staking)
        .with_value(150_000_000.try_into().unwrap())
        .with_fee(100.try_into().unwrap())
        .with_network_id(NetworkId::Dummy)
        .with_validity_start_height(1)
        .with_recipient(recipient);
    let mut proof_builder = tx_builder.generate().unwrap().unwrap_staking();
    proof_builder.drop_validator(&bls_pair);
    let tx2 = proof_builder.generate().unwrap();

    assert_eq!(tx2, tx);
}

fn bls_key_pair() -> BlsKeyPair {
    const BLS_PRIVKEY: &str =
        "93ded88af373537a2fad738892ae29cf012bb27875cb66af9278991acbcb8e44f414\
    9c27fe9d62a31ae8537fc4891e935e1303c511091095c0ad083a1cfc0f5f223c394c2d5109288e639cde0692facc9fd\
    221a806c0003835db99b423360000";
    BlsKeyPair::from_secret(
        &Deserialize::deserialize(&mut &hex::decode(BLS_PRIVKEY).unwrap()[..]).unwrap(),
    )
}

fn ed25519_key_pair() -> KeyPair {
    let priv_key: PrivateKey =
        Deserialize::deserialize(&mut &hex::decode(STAKER_PRIVATE_KEY).unwrap()[..]).unwrap();
    priv_key.into()
}

fn make_incoming_transaction(data: IncomingStakingTransactionData, value: u64) -> Transaction {
    match data {
        IncomingStakingTransactionData::Stake { .. }
        | IncomingStakingTransactionData::CreateValidator { .. } => Transaction::new_extended(
            Address::from_any_str(STAKER_ADDRESS).unwrap(),
            AccountType::Basic,
            Address::from([1u8; 20]),
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
            Address::from([1u8; 20]),
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
    bls_pair: &BlsKeyPair,
    key_pair: &KeyPair,
) -> Transaction {
    let mut tx = make_incoming_transaction(data, value);
    tx.data = IncomingStakingTransactionData::set_validator_signature_on_data(
        &tx.data,
        bls_pair.sign(&tx.serialize_content()).compress(),
    )
    .unwrap();

    tx.proof = SignatureProof::from(
        key_pair.public.clone(),
        key_pair.sign(&tx.serialize_content()),
    )
    .serialize_to_vec();
    tx
}

fn make_unstake_transaction(key_pair: &KeyPair, value: u64) -> Transaction {
    let mut tx = Transaction::new_extended(
        Address::from([1u8; 20]),
        AccountType::Staking,
        Address::from_any_str(STAKER_ADDRESS).unwrap(),
        AccountType::Basic,
        value.try_into().unwrap(),
        100.try_into().unwrap(),
        vec![],
        1,
        NetworkId::Dummy,
    );
    let proof = OutgoingStakingTransactionProof::Unstake(SignatureProof::from(
        key_pair.public.clone(),
        key_pair.sign(&tx.serialize_content()),
    ));
    tx.proof = proof.serialize_to_vec();
    tx
}

fn make_drop_transaction(key_pair: &BlsKeyPair, value: u64) -> Transaction {
    let mut tx = Transaction::new_extended(
        Address::from([1u8; 20]),
        AccountType::Staking,
        Address::from_any_str(STAKER_ADDRESS).unwrap(),
        AccountType::Basic,
        value.try_into().unwrap(),
        100.try_into().unwrap(),
        vec![],
        1,
        NetworkId::Dummy,
    );
    let proof = OutgoingStakingTransactionProof::DropValidator {
        validator_key: key_pair.public_key.compress(),
        signature: key_pair.sign(&tx.serialize_content()).compress(),
    };
    tx.proof = proof.serialize_to_vec();
    tx
}

fn make_self_transaction(data: SelfStakingTransactionData, value: u64) -> Transaction {
    let mut tx = Transaction::new_extended(
        Address::from([1u8; 20]),
        AccountType::Staking,
        Address::from([1u8; 20]),
        AccountType::Staking,
        value.try_into().unwrap(),
        100.try_into().unwrap(),
        data.serialize_to_vec(),
        1,
        NetworkId::Dummy,
    );
    let private_key =
        PrivateKey::deserialize_from_vec(&hex::decode(STAKER_PRIVATE_KEY).unwrap()).unwrap();
    let key_pair = KeyPair::from(private_key);
    tx.proof = SignatureProof::from(
        key_pair.public.clone(),
        key_pair.sign(&tx.serialize_content()),
    )
    .serialize_to_vec();
    tx
}
