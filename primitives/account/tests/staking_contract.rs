use std::convert::TryInto;

use rand::thread_rng;

use beserial::{Deserialize, Serialize};
use nimiq_account::{
    AccountError, AccountInherentInteraction, AccountTransactionInteraction, Inherent,
    InherentType, StakingContract,
};
use nimiq_bls::CompressedPublicKey as BlsPublicKey;
use nimiq_bls::KeyPair as BlsKeyPair;
use nimiq_bls::SecretKey as BlsSecretKey;
use nimiq_bls::Signature as BlsSignature;
use nimiq_hash::{Blake2bHash, Hash};
use nimiq_keys::{Address, KeyPair, PrivateKey};
use nimiq_primitives::account::{AccountType, ValidatorId};
use nimiq_primitives::coin::Coin;
use nimiq_primitives::networks::NetworkId;
use nimiq_primitives::slots::SlashedSlot;
use nimiq_transaction::account::staking_contract::{
    IncomingStakingTransactionData, OutgoingStakingTransactionProof, SelfStakingTransactionData,
};
use nimiq_transaction::account::AccountTransactionVerification;
use nimiq_transaction::{SignatureProof, Transaction, TransactionError};
use nimiq_utils::key_rng::SecureGenerate;

const CONTRACT_1: &str = "00000000000000000000000000000000000000000000000000000000000000000000";
const CONTRACT_2: &str =
    "0000000023c346000000000155555555555555555555555555555555555555550000000023c346000303030303030303030303030303030303030303003d4e4eb0fa2fee42501368dc41115f64741e9d9496bbc2fe4cfd407f10272eef87b839d6e25b0eb7338427d895e4209190b6c5aa580f134693623a30ebafdaf95a268b3b84a840fc45d06283d71fe4faa2c7d08cd431bbda165c53a50453015a49ca120626991ff9558be65a7958158387829d6e56e2861e80b85e8c795d93f907afb19e6e2e5aaed9a3158eac5a035189986ff5803dd18fa02bdf5535e5495ed96990665ec165b3ba86fc1a7f7dabeb0510e1823813bf5ab1a01b4fff00bcd0373bc265efa135f8755ebae72b645a890d27ce8af31417347bc3a1d9cf09db339b68d1c9a50bb9c00faeedbefe9bab5a63b580e5f79c4a30dc1bdacccec0fc6a08e0853518e88557001a612d4c30d2fbc2a126a066a94f299ac5ce61000000020202020202020202020202020202020202020202000000000bebc2005e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e0000000005f5e10000000000000000000000000000000000000000000000";
const VALIDATOR_ID: &str = "5555555555555555555555555555555555555555";
const VALIDATOR_KEY: &str = "003d4e4eb0fa2fee42501368dc41115f64741e9d9496bbc2fe4cfd407f10272eef87b839d6e25b0eb7338427d895e4209190b6c5aa580f134693623a30ebafdaf95a268b3b84a840fc45d06283d71fe4faa2c7d08cd431bbda165c53a50453015a49ca120626991ff9558be65a7958158387829d6e56e2861e80b85e8c795d93f907afb19e6e2e5aaed9a3158eac5a035189986ff5803dd18fa02bdf5535e5495ed96990665ec165b3ba86fc1a7f7dabeb0510e1823813bf5ab1a01b4fff00bcd0373bc265efa135f8755ebae72b645a890d27ce8af31417347bc3a1d9cf09db339b68d1c9a50bb9c00faeedbefe9bab5a63b580e5f79c4a30dc1bdacccec0fc6a08e0853518e88557001a612d4c30d2fbc2a126a066a94f299ac5ce61";
const VALIDATOR_SECRET_KEY: &str =
    "b552baff2c2cc4937ec3531c833c3ffc08f92a95b3ba4a53cf7e8c99ef9db99b99559b8dbb8f3c44fa5671da42cc2633759aea71c1b696ea18df5451d0d43a225a882b29a1091ece16e82f664c2c6f2b360c7b6ce84e5d0995ae45290dbd0000";
const STAKER_ADDRESS: &str = "9cd82948650d902d95d52ea2ec91eae6deb0c9fe";
const STAKER_PRIVATE_KEY: &str = "b410a7a583cbc13ef4f1cbddace30928bcb4f9c13722414bc4a2faaba3f4e187";

// The following code is kept as a reference on how to generate the data for the CONTRACT_2 constant.
// #[test]
// fn generate_contract() {
//     let key_pair =
//         BlsKeyPair::deserialize_from_vec(&hex::decode(VALIDATOR_SECRET_KEY).unwrap()).unwrap();
//     let mut contract = StakingContract {
//         balance: Default::default(),
//         active_validators_sorted: Default::default(),
//         active_validators_by_key: Default::default(),
//         inactive_validators_by_key: Default::default(),
//         current_epoch_parking: Default::default(),
//         previous_epoch_parking: Default::default(),
//         current_lost_rewards: Default::default(),
//         previous_lost_rewards: Default::default(),
//         current_disabled_slots: Default::default(),
//         previous_disabled_slots: Default::default(),
//         inactive_stake_by_address: Default::default(),
//     };
//     contract.create_validator(
//         key_pair.public_key.compress(),
//         Address::from([3u8; 20]),
//         300_000_000.try_into().unwrap(),
//     );
//     contract.stake(
//         Address::from([2u8; 20]),
//         200_000_000.try_into().unwrap(),
//         &key_pair.public_key.compress(),
//     );
//     contract.stake(
//         Address::from([0x5eu8; 20]),
//         100_000_000.try_into().unwrap(),
//         &key_pair.public_key.compress(),
//     );
//     assert_eq!(&hex::encode(contract.serialize_to_vec()), "");
// }

#[test]
fn it_can_de_serialize_a_staking_contract() {
    let bytes_1: Vec<u8> = hex::decode(CONTRACT_1).unwrap();
    let contract_1: StakingContract = Deserialize::deserialize(&mut &bytes_1[..]).unwrap();
    assert_eq!(contract_1.balance, 0.try_into().unwrap());
    assert_eq!(contract_1.active_validators_by_id.len(), 0);
    assert_eq!(contract_1.active_validators_sorted.len(), 0);
    assert_eq!(contract_1.inactive_validators_by_id.len(), 0);
    assert_eq!(contract_1.current_epoch_parking.len(), 0);
    assert_eq!(contract_1.previous_epoch_parking.len(), 0);
    assert_eq!(contract_1.inactive_stake_by_address.len(), 0);
    let mut bytes_1_out = Vec::<u8>::with_capacity(contract_1.serialized_size());
    let size_1_out = contract_1.serialize(&mut bytes_1_out).unwrap();
    assert_eq!(size_1_out, contract_1.serialized_size());
    assert_eq!(hex::encode(bytes_1_out), CONTRACT_1);

    let validator_id =
        ValidatorId::deserialize_from_vec(&hex::decode(VALIDATOR_ID).unwrap()).unwrap();

    let bytes_2: Vec<u8> = hex::decode(CONTRACT_2).unwrap();
    let contract_2: StakingContract = Deserialize::deserialize(&mut &bytes_2[..]).unwrap();
    assert_eq!(contract_2.balance, 600_000_000.try_into().unwrap());
    let validator = contract_2
        .get_validator(&validator_id)
        .expect("Validator missing");
    assert_eq!(validator.id, validator_id);
    assert_eq!(validator.balance, Coin::from_u64_unchecked(600_000_000));
    assert_eq!(validator.active_stake_by_address.read().len(), 2);
    assert_eq!(
        contract_2.get_active_stake(&validator_id, &Address::from([2u8; 20])),
        Some(Coin::from_u64_unchecked(200_000_000u64))
    );
    assert_eq!(
        contract_2.get_active_stake(&validator_id, &Address::from([0x5eu8; 20])),
        Some(Coin::from_u64_unchecked(100_000_000u64))
    );
    assert_eq!(contract_2.active_validators_by_id.len(), 1);
    assert_eq!(contract_2.active_validators_sorted.len(), 1);
    assert_eq!(contract_2.inactive_validators_by_id.len(), 0);
    assert_eq!(contract_2.current_epoch_parking.len(), 0);
    assert_eq!(contract_2.previous_epoch_parking.len(), 0);
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
        sender,
        AccountType::Basic,
        AccountType::Staking,
        100.try_into().unwrap(),
        0.try_into().unwrap(),
        0,
        NetworkId::Dummy,
    );

    assert_eq!(
        AccountType::verify_incoming_transaction(&transaction),
        Err(TransactionError::InvalidForRecipient)
    );
}

#[test]
fn it_can_verify_validator_and_staking_transaction() {
    let validator_key =
        BlsPublicKey::deserialize_from_vec(&hex::decode(VALIDATOR_KEY).unwrap()).unwrap();
    let validator_id: ValidatorId = [0u8; 20].into();
    let keypair = bls_key_pair();

    let mut tx = make_incoming_transaction(
        IncomingStakingTransactionData::CreateValidator {
            validator_key: validator_key.clone(),
            proof_of_knowledge: keypair.sign(&validator_key.serialize_to_vec()).compress(),
            reward_address: Address::from([3u8; 20]),
        },
        100_000_000,
    );

    // Valid
    assert_eq!(AccountType::verify_incoming_transaction(&tx), Ok(()));

    // Below minimum stake
    tx.value = 1.try_into().unwrap();
    assert_eq!(
        AccountType::verify_incoming_transaction(&tx),
        Err(TransactionError::InvalidForRecipient)
    );

    // Invalid proof of knowledge
    let other_pair = BlsKeyPair::generate(&mut thread_rng());
    let invalid_pok = other_pair.sign(&keypair.public_key);
    let tx = make_incoming_transaction(
        IncomingStakingTransactionData::CreateValidator {
            validator_key,
            proof_of_knowledge: invalid_pok.compress(),
            reward_address: Address::from([3u8; 20]),
        },
        100_000_000,
    );
    assert_eq!(
        AccountType::verify_incoming_transaction(&tx),
        Err(TransactionError::InvalidData)
    );

    // Staking
    let mut tx = make_incoming_transaction(
        IncomingStakingTransactionData::Stake {
            validator_id,
            staker_address: None,
        },
        100_000_000,
    );

    // Valid
    assert_eq!(AccountType::verify_incoming_transaction(&tx), Ok(()));

    // Below minimum stake
    tx.value = 0.try_into().unwrap();
    assert_eq!(
        AccountType::verify_incoming_transaction(&tx),
        Err(TransactionError::InvalidForRecipient)
    );
}

#[test]
fn it_can_apply_validator_and_staking_transaction() {
    let mut contract = make_empty_contract();

    let bls_pair = bls_key_pair();
    let proof_of_knowledge = bls_pair.sign(&bls_pair.public_key).compress();
    let staker_address = Address::from_any_str(STAKER_ADDRESS).unwrap();

    // Create validator
    let tx_1 = make_incoming_transaction(
        IncomingStakingTransactionData::CreateValidator {
            validator_key: bls_pair.public_key.compress(),
            proof_of_knowledge,
            reward_address: Default::default(),
        },
        150_000_000,
    );
    let validator_id: ValidatorId = tx_1.hash::<Blake2bHash>().as_slice()[0..20].into();

    assert_eq!(
        StakingContract::check_incoming_transaction(&tx_1, 2, 0),
        Ok(())
    );
    assert_eq!(contract.commit_incoming_transaction(&tx_1, 2, 0), Ok(None));
    assert_eq!(contract.active_validators_by_id.len(), 1);
    assert_eq!(
        contract.get_validator(&validator_id).unwrap().balance,
        Coin::from_u64_unchecked(150_000_000)
    );
    assert_eq!(contract.balance, 150_000_000.try_into().unwrap());

    // Cannot overwrite validator
    assert_eq!(
        contract.commit_incoming_transaction(&tx_1, 2, 0),
        Err(AccountError::InvalidForRecipient)
    );

    // Stake
    let tx_2 = make_incoming_transaction(
        IncomingStakingTransactionData::Stake {
            validator_id: validator_id.clone(),
            staker_address: None,
        },
        150_000_000,
    );
    assert_eq!(
        StakingContract::check_incoming_transaction(&tx_2, 3, 0),
        Ok(())
    );
    assert_eq!(contract.commit_incoming_transaction(&tx_2, 3, 0), Ok(None));
    assert_eq!(contract.active_validators_by_id.len(), 1);
    assert_eq!(
        contract.get_validator(&validator_id).unwrap().balance,
        Coin::from_u64_unchecked(300_000_000)
    );
    assert_eq!(
        contract
            .get_active_stake(&validator_id, &staker_address)
            .unwrap(),
        Coin::from_u64_unchecked(150_000_000)
    );
    assert_eq!(contract.balance, 300_000_000.try_into().unwrap());

    // Stake again, for different address
    let tx_3 = make_incoming_transaction(
        IncomingStakingTransactionData::Stake {
            validator_id: validator_id.clone(),
            staker_address: Some(Address::from([2u8; 20])),
        },
        150_000_000,
    );
    assert_eq!(
        StakingContract::check_incoming_transaction(&tx_3, 4, 0),
        Ok(())
    );
    assert_eq!(contract.commit_incoming_transaction(&tx_3, 4, 0), Ok(None));
    assert_eq!(contract.active_validators_by_id.len(), 1);
    assert_eq!(
        contract.get_validator(&validator_id).unwrap().balance,
        Coin::from_u64_unchecked(450_000_000)
    );
    assert_eq!(
        contract
            .get_active_stake(&validator_id, &staker_address)
            .unwrap(),
        Coin::from_u64_unchecked(150_000_000)
    );
    assert_eq!(
        contract
            .get_active_stake(&validator_id, &Address::from([2u8; 20]))
            .unwrap(),
        Coin::from_u64_unchecked(150_000_000)
    );
    assert_eq!(contract.balance, 450_000_000.try_into().unwrap());

    // Stake more
    let tx_4 = make_incoming_transaction(
        IncomingStakingTransactionData::Stake {
            validator_id: validator_id.clone(),
            staker_address: None,
        },
        150_000_000,
    );
    assert_eq!(
        StakingContract::check_incoming_transaction(&tx_4, 5, 0),
        Ok(())
    );
    assert_eq!(contract.commit_incoming_transaction(&tx_4, 5, 0), Ok(None));
    assert_eq!(contract.active_validators_by_id.len(), 1);
    assert_eq!(
        contract.get_validator(&validator_id).unwrap().balance,
        Coin::from_u64_unchecked(600_000_000)
    );
    assert_eq!(
        contract
            .get_active_stake(&validator_id, &staker_address)
            .unwrap(),
        Coin::from_u64_unchecked(300_000_000)
    );
    assert_eq!(
        contract
            .get_active_stake(&validator_id, &Address::from([2u8; 20]))
            .unwrap(),
        Coin::from_u64_unchecked(150_000_000)
    );
    assert_eq!(contract.balance, 600_000_000.try_into().unwrap());

    // Revert everything
    assert_eq!(
        contract.revert_incoming_transaction(&tx_4, 5, 0, None),
        Ok(())
    );
    assert_eq!(contract.balance, 450_000_000.try_into().unwrap());
    assert_eq!(
        contract.revert_incoming_transaction(&tx_3, 4, 0, None),
        Ok(())
    );
    assert_eq!(contract.balance, 300_000_000.try_into().unwrap());
    assert_eq!(
        contract.revert_incoming_transaction(&tx_2, 3, 0, None),
        Ok(())
    );
    assert_eq!(contract.active_validators_by_id.len(), 1);
    assert_eq!(contract.balance, 150_000_000.try_into().unwrap());
    assert_eq!(
        contract.revert_incoming_transaction(&tx_1, 2, 0, None),
        Ok(())
    );
    assert_eq!(contract.active_validators_by_id.len(), 0);
    assert_eq!(contract.balance, 0.try_into().unwrap());
}

/// Only called with outgoing/self transactions.
fn test_proof_verification(transaction: Transaction) {
    // No proof
    let mut tx_1 = transaction.clone();
    tx_1.proof = vec![];
    assert!(AccountType::verify_outgoing_transaction(&tx_1).is_err());

    // Invalid proof
    // We need to construct seemingly valid looking proofs.
    // For that, we need to know the structure.
    let mut tx_3 = transaction.clone();

    let key_pair = ed25519_key_pair();
    let other_pair = KeyPair::from(
        PrivateKey::deserialize_from_vec(
            &hex::decode("5d205b80adbffc32fe21927c7fd119623d44746a3ddd35a299a1e483c3402cd9")
                .unwrap(),
        )
        .unwrap(),
    );

    if transaction.recipient_type != AccountType::Staking {
        // More complex proof.
        let data: OutgoingStakingTransactionProof =
            Deserialize::deserialize_from_vec(&transaction.proof[..]).unwrap();
        match data {
            OutgoingStakingTransactionProof::Unstake(_) => {
                tx_3.proof = OutgoingStakingTransactionProof::Unstake(SignatureProof::from(
                    key_pair.public,
                    other_pair.sign(&tx_3.serialize_content()),
                ))
                .serialize_to_vec();
            }
            OutgoingStakingTransactionProof::DropValidator {
                validator_id,
                validator_key,
                ..
            } => {
                let bls_pair = BlsKeyPair::from(
                    BlsSecretKey::deserialize_from_vec(
                        &hex::decode(
                            "3a12b799f4fa06f1d777695c8ceeeb3fcd831d3342d1f1c7046b9359888bb4e31\
                            304d72e819fb685502bad8a7876c7522e4abdf1bd2f4a55306164029e74922a4dd85895\
                            b9e54d45139d22041e83f477eb8b489c19a41aa0529b8add804d0000",
                        )
                        .unwrap(),
                    )
                    .unwrap(),
                );

                let proof = OutgoingStakingTransactionProof::DropValidator {
                    validator_id,
                    validator_key,
                    signature: bls_pair.sign(&tx_3.serialize_content()).compress(),
                };
                tx_3.proof = proof.serialize_to_vec();
            }
        }
    } else {
        // Simple signature proof.
        tx_3.proof =
            SignatureProof::from(key_pair.public, other_pair.sign(&tx_3.serialize_content()))
                .serialize_to_vec();
    }

    assert_eq!(
        AccountType::verify_outgoing_transaction(&tx_3),
        Err(TransactionError::InvalidProof)
    );
    if transaction.recipient_type == AccountType::Staking {
        assert_eq!(AccountType::verify_incoming_transaction(&tx_3), Ok(()));
    }

    // Valid
    assert_eq!(
        AccountType::verify_outgoing_transaction(&transaction),
        Ok(())
    );
    if transaction.recipient_type == AccountType::Staking {
        assert_eq!(
            AccountType::verify_incoming_transaction(&transaction),
            Ok(())
        );
    }
}

#[test]
fn it_can_verify_self_transactions() {
    let validator_id: ValidatorId = [0u8; 20].into();
    test_proof_verification(make_self_transaction(
        SelfStakingTransactionData::RetireStake(validator_id.clone()),
        10,
    ));
    test_proof_verification(make_self_transaction(
        SelfStakingTransactionData::ReactivateStake(validator_id),
        10,
    ));
}

#[test]
fn it_can_apply_retiring_transaction() {
    let key_pair = ed25519_key_pair();
    let staker_address = Address::from(&key_pair);
    let bls_pair = bls_key_pair();
    let validator_id: ValidatorId = [0u8; 20].into();
    let mut contract = make_sample_contract(&validator_id, &key_pair, &bls_pair);

    // Retire first half of stake
    let tx_1 = make_self_transaction(
        SelfStakingTransactionData::RetireStake(validator_id.clone()),
        99_999_900,
    );
    assert_eq!(contract.check_outgoing_transaction(&tx_1, 2, 0), Ok(()));
    assert_eq!(contract.commit_outgoing_transaction(&tx_1, 2, 0), Ok(None));
    assert_eq!(
        StakingContract::check_incoming_transaction(&tx_1, 2, 0),
        Ok(())
    );
    assert_eq!(contract.commit_incoming_transaction(&tx_1, 2, 0), Ok(None));

    assert_eq!(
        contract.get_active_stake(&validator_id, &staker_address),
        Some(Coin::from_u64_unchecked(50_000_000))
    );
    assert_eq!(contract.inactive_stake_by_address.len(), 1);
    assert_eq!(
        contract
            .inactive_stake_by_address
            .get(&staker_address)
            .unwrap()
            .balance,
        Coin::from_u64_unchecked(99_999_900)
    );
    assert_eq!(contract.balance, Coin::from_u64_unchecked(299_999_900));

    // Try to retire too much stake
    let tx_2 = make_self_transaction(
        SelfStakingTransactionData::RetireStake(validator_id.clone()),
        99_999_900,
    );
    let funds_error = AccountError::InsufficientFunds {
        needed: Coin::from_u64_unchecked(100_000_000),
        balance: Coin::from_u64_unchecked(50_000_000),
    };
    assert_eq!(
        contract
            .check_outgoing_transaction(&tx_2, 3, 0)
            .expect_err("Should error"),
        funds_error
    );
    assert_eq!(
        contract
            .commit_outgoing_transaction(&tx_2, 3, 0)
            .expect_err("Should error"),
        funds_error
    );

    // Retire second half of stake in two transactions
    let tx_3 = make_self_transaction(
        SelfStakingTransactionData::RetireStake(validator_id.clone()),
        24_999_900,
    );
    assert_eq!(contract.check_outgoing_transaction(&tx_3, 3, 0), Ok(()));
    assert_eq!(contract.commit_outgoing_transaction(&tx_3, 3, 0), Ok(None));
    assert_eq!(contract.check_outgoing_transaction(&tx_3, 3, 0), Ok(()));
    assert_eq!(contract.commit_outgoing_transaction(&tx_3, 3, 0), Ok(None));
    assert_eq!(
        StakingContract::check_incoming_transaction(&tx_3, 3, 0),
        Ok(())
    );
    let receipt_incoming_1 = contract
        .commit_incoming_transaction(&tx_3, 3, 0)
        .unwrap()
        .unwrap();
    assert_eq!(
        StakingContract::check_incoming_transaction(&tx_3, 3, 0),
        Ok(())
    );
    let receipt_incoming_2 = contract
        .commit_incoming_transaction(&tx_3, 3, 0)
        .unwrap()
        .unwrap();

    assert_eq!(
        contract.get_active_stake(&validator_id, &staker_address),
        None
    );
    assert_eq!(contract.inactive_stake_by_address.len(), 1);
    assert_eq!(
        contract
            .inactive_stake_by_address
            .get(&staker_address)
            .unwrap()
            .balance,
        Coin::from_u64_unchecked(149_999_700)
    );
    assert_eq!(contract.balance, Coin::from_u64_unchecked(299_999_700));

    // Try to retire nonexistent funds
    assert_eq!(
        contract.check_outgoing_transaction(&tx_3, 4, 0),
        Err(AccountError::InvalidForSender)
    );
    assert_eq!(
        contract.commit_outgoing_transaction(&tx_3, 4, 0),
        Err(AccountError::InvalidForSender)
    );

    // Revert to original state
    assert_eq!(
        contract.revert_incoming_transaction(&tx_3, 3, 0, Some(&receipt_incoming_2)),
        Ok(())
    );
    assert_eq!(
        contract.revert_incoming_transaction(&tx_3, 3, 0, Some(&receipt_incoming_1)),
        Ok(())
    );
    assert_eq!(
        contract.revert_outgoing_transaction(&tx_3, 3, 0, None),
        Ok(())
    );
    assert_eq!(
        contract.revert_outgoing_transaction(&tx_3, 3, 0, None),
        Ok(())
    );
    assert_eq!(
        contract.revert_incoming_transaction(&tx_1, 2, 0, None),
        Ok(())
    );
    assert_eq!(
        contract.revert_outgoing_transaction(&tx_1, 2, 0, None),
        Ok(())
    );

    assert_eq!(
        contract.get_active_stake(&validator_id, &staker_address),
        Some(Coin::from_u64_unchecked(150_000_000))
    );
    assert_eq!(contract.inactive_stake_by_address.len(), 0);
    assert_eq!(contract.balance, Coin::from_u64_unchecked(300_000_000));
}

#[test]
fn it_can_verify_unstaking_and_drop_transactions() {
    let key_pair = ed25519_key_pair();
    let bls_pair = bls_key_pair();
    let validator_id: ValidatorId = [0u8; 20].into();

    test_proof_verification(make_unstake_transaction(&key_pair, 10));
    test_proof_verification(make_drop_transaction(validator_id, &bls_pair, 10));
}

#[test]
fn it_can_apply_unstaking_transaction() {
    let key_pair = ed25519_key_pair();
    let recipient = Address::from(&key_pair);
    let bls_pair = bls_key_pair();
    let validator_id: ValidatorId = [0u8; 20].into();
    let mut contract = make_sample_contract(&validator_id, &key_pair, &bls_pair);

    // Block 2: Retire first half of stake
    let tx_1 = make_self_transaction(
        SelfStakingTransactionData::RetireStake(validator_id.clone()),
        50_000_000,
    );
    assert_eq!(contract.commit_outgoing_transaction(&tx_1, 2, 0), Ok(None));
    assert_eq!(contract.commit_incoming_transaction(&tx_1, 2, 0), Ok(None));
    assert_eq!(contract.balance, Coin::from_u64_unchecked(299_999_900));
    assert_eq!(
        contract.get_active_stake(&validator_id, &recipient),
        Some(Coin::from_u64_unchecked(99_999_900))
    );
    assert_eq!(
        contract
            .inactive_stake_by_address
            .get(&recipient)
            .unwrap()
            .balance,
        Coin::from_u64_unchecked(50_000_000)
    );

    // Try to unstake too much
    let tx_2 = make_unstake_transaction(&key_pair, 999_999_899);
    let funds_error = AccountError::InsufficientFunds {
        needed: Coin::from_u64_unchecked(999_999_999),
        balance: Coin::from_u64_unchecked(50_000_000),
    };
    assert_eq!(
        contract
            .check_outgoing_transaction(&tx_2, 40003, 0)
            .expect_err("Should error"),
        funds_error
    );
    assert_eq!(
        contract
            .commit_outgoing_transaction(&tx_2, 40003, 0)
            .expect_err("Should error"),
        funds_error
    );

    // Block 40003: Unstake quarter
    let tx_3 = make_unstake_transaction(&key_pair, 12_500_000 - 100);
    assert_eq!(contract.check_outgoing_transaction(&tx_3, 40003, 0), Ok(()));
    assert_eq!(
        contract.commit_outgoing_transaction(&tx_3, 40003, 0),
        Ok(None)
    );
    assert_eq!(contract.balance, Coin::from_u64_unchecked(287_499_900));
    assert_eq!(
        contract.get_active_stake(&validator_id, &recipient),
        Some(Coin::from_u64_unchecked(99_999_900))
    );
    assert_eq!(
        contract
            .inactive_stake_by_address
            .get(&recipient)
            .unwrap()
            .balance,
        Coin::from_u64_unchecked(37_500_000)
    );

    // Block 40004: Unstake another quarter
    let tx_4 = tx_3.clone();
    assert_eq!(contract.check_outgoing_transaction(&tx_4, 40004, 0), Ok(()));
    assert_eq!(
        contract.commit_outgoing_transaction(&tx_4, 40004, 0),
        Ok(None)
    );
    assert_eq!(contract.balance, Coin::from_u64_unchecked(274_999_900));
    assert_eq!(
        contract.get_active_stake(&validator_id, &recipient),
        Some(Coin::from_u64_unchecked(99_999_900))
    );
    assert_eq!(
        contract
            .inactive_stake_by_address
            .get(&recipient)
            .unwrap()
            .balance,
        Coin::from_u64_unchecked(25_000_000)
    );

    // Revert block 40004
    assert_eq!(
        contract.revert_outgoing_transaction(&tx_4, 40004, 0, None),
        Ok(())
    );
    assert_eq!(contract.balance, Coin::from_u64_unchecked(287_499_900));
    assert_eq!(
        contract.get_active_stake(&validator_id, &recipient),
        Some(Coin::from_u64_unchecked(99_999_900))
    );
    assert_eq!(
        contract
            .inactive_stake_by_address
            .get(&recipient)
            .unwrap()
            .balance,
        Coin::from_u64_unchecked(37_500_000)
    );

    // New block 40004: Retire second half of stake
    let tx_5 = make_self_transaction(
        SelfStakingTransactionData::RetireStake(validator_id.clone()),
        99_999_800,
    );
    assert_eq!(
        contract.commit_outgoing_transaction(&tx_5, 40004, 0),
        Ok(None)
    );
    let receipt_5_incoming = contract
        .commit_incoming_transaction(&tx_5, 40004, 0)
        .unwrap()
        .unwrap();
    assert_eq!(contract.balance, Coin::from_u64_unchecked(287_499_800));
    assert_eq!(contract.get_active_stake(&validator_id, &recipient), None);
    assert_eq!(
        contract
            .inactive_stake_by_address
            .get(&recipient)
            .unwrap()
            .balance,
        Coin::from_u64_unchecked(137_499_800)
    );

    // Try to replay reverted unstaking, should fail
    let tx_6 = tx_3.clone();
    assert_eq!(
        contract.check_outgoing_transaction(&tx_6, 40005, 0),
        Err(AccountError::InvalidForSender)
    );
    assert_eq!(
        contract.commit_outgoing_transaction(&tx_6, 40005, 0),
        Err(AccountError::InvalidForSender)
    );

    // Unstake rest
    let tx_7 = make_unstake_transaction(&key_pair, 137_499_700);
    assert_eq!(
        contract.check_outgoing_transaction(&tx_7, 100000, 0),
        Ok(())
    );
    let receipt_7 = contract
        .commit_outgoing_transaction(&tx_7, 100000, 0)
        .unwrap()
        .unwrap();

    // Contract is empty at this point (except initial validator stake)
    assert_eq!(contract.balance, Coin::from_u64_unchecked(150_000_000));
    assert_eq!(contract.inactive_stake_by_address.len(), 0);

    // Try to unstake nonexistent funds
    let tx_8 = tx_3.clone();
    assert_eq!(
        contract.check_outgoing_transaction(&tx_8, 40006, 0),
        Err(AccountError::InvalidForSender)
    );
    assert_eq!(
        contract.commit_outgoing_transaction(&tx_8, 40006, 0),
        Err(AccountError::InvalidForSender)
    );

    // Revert everything
    assert_eq!(
        contract.revert_outgoing_transaction(&tx_7, 100000, 0, Some(&receipt_7)),
        Ok(())
    );
    assert_eq!(
        contract.revert_incoming_transaction(&tx_5, 40004, 0, Some(&receipt_5_incoming)),
        Ok(())
    );
    assert_eq!(
        contract.revert_outgoing_transaction(&tx_5, 40004, 0, None),
        Ok(())
    );
    assert_eq!(
        contract.revert_outgoing_transaction(&tx_3, 40003, 0, None),
        Ok(())
    );
    assert_eq!(
        contract.revert_incoming_transaction(&tx_1, 2, 0, None),
        Ok(())
    );
    assert_eq!(
        contract.revert_outgoing_transaction(&tx_1, 2, 0, None),
        Ok(())
    );

    // Initial contract state
    assert_eq!(contract.active_validators_by_id.len(), 1);
    assert_eq!(contract.inactive_stake_by_address.len(), 0);
    assert_eq!(contract.balance, Coin::from_u64_unchecked(300_000_000));
    assert_eq!(
        contract.get_active_stake(&validator_id, &recipient),
        Some(Coin::from_u64_unchecked(150_000_000))
    );
}

#[test]
fn it_can_apply_rededicate_stake_tx() {
    let validator_id1: ValidatorId = [1u8; 20].into();
    let validator_id2: ValidatorId = [2u8; 20].into();

    let bls_pair1 = BlsKeyPair::generate_default_csprng();
    let bls_pair2 = BlsKeyPair::generate_default_csprng();

    let mut contract = make_empty_contract();
    contract
        .create_validator(
            validator_id1.clone(),
            bls_pair1.public_key.compress(),
            [1; 20].into(),
            Coin::from_u64_unchecked(0),
        )
        .unwrap();
    contract
        .create_validator(
            validator_id2.clone(),
            bls_pair2.public_key.compress(),
            [2; 20].into(),
            Coin::from_u64_unchecked(0),
        )
        .unwrap();
    contract
        .stake(
            Address::from(&ed25519_key_pair()),
            Coin::from_u64_unchecked(150000000),
            &validator_id1,
        )
        .unwrap();

    let tx_1 = make_self_transaction(
        SelfStakingTransactionData::RededicateStake {
            from_validator_id: validator_id1.clone(),
            to_validator_id: validator_id2.clone(),
        },
        50000000,
    );
    assert_eq!(contract.check_outgoing_transaction(&tx_1, 2, 0), Ok(()));
    assert_eq!(contract.commit_outgoing_transaction(&tx_1, 2, 0), Ok(None));
    assert_eq!(
        StakingContract::check_incoming_transaction(&tx_1, 2, 0),
        Ok(())
    );
    assert_eq!(contract.commit_incoming_transaction(&tx_1, 2, 0), Ok(None));

    // initial balance - moved stake - fees
    assert_eq!(
        contract.get_validator(&validator_id1).unwrap().balance,
        Coin::from_u64_unchecked(150000000 - 50000000 - 100)
    );
    assert_eq!(
        contract.get_validator(&validator_id2).unwrap().balance,
        Coin::from_u64_unchecked(50000000)
    );

    // revert transaction
    assert_eq!(
        contract.revert_outgoing_transaction(&tx_1, 2, 0, None),
        Ok(())
    );
    assert_eq!(
        contract.revert_incoming_transaction(&tx_1, 2, 0, None),
        Ok(())
    );

    assert_eq!(
        contract.get_validator(&validator_id1).unwrap().balance,
        Coin::from_u64_unchecked(150000000)
    );
    assert_eq!(
        contract.get_validator(&validator_id2).unwrap().balance,
        Coin::from_u64_unchecked(0)
    );
}

#[test]
fn it_can_verify_inherent() {
    let key_pair = ed25519_key_pair();
    let bls_pair = bls_key_pair();
    let mut contract = make_sample_contract(&ValidatorId::default(), &key_pair, &bls_pair);

    // Reward inherent
    let inherent_1 = Inherent {
        ty: InherentType::Reward,
        target: Address::from([0u8; 20]),
        value: Coin::ZERO,
        data: Vec::new(),
    };
    assert_eq!(
        contract.check_inherent(&inherent_1, 0, 0),
        Err(AccountError::InvalidForTarget)
    );
    assert_eq!(
        contract.commit_inherent(&inherent_1, 0, 0),
        Err(AccountError::InvalidForTarget)
    );
}

#[test]
fn it_rejects_invalid_slash_inherents() {
    let bls_pair = bls_key_pair();
    let key_pair = ed25519_key_pair();
    let validator_id: ValidatorId = ValidatorId::default();
    let mut contract = make_sample_contract(&validator_id, &key_pair, &bls_pair);
    let slot = SlashedSlot {
        slot: 0,
        validator_id,
        event_block: 0,
    };

    // Invalid inherent
    let mut inherent = Inherent {
        ty: InherentType::Slash,
        target: Default::default(),
        value: Coin::from_u64_unchecked(1),
        data: slot.serialize_to_vec(),
    };

    // Invalid value.
    assert_eq!(
        contract.check_inherent(&inherent, 0, 0),
        Err(AccountError::InvalidInherent)
    );
    assert_eq!(
        contract.commit_inherent(&inherent, 0, 0),
        Err(AccountError::InvalidInherent)
    );

    // Invalid data.
    inherent.value = Coin::ZERO;
    inherent.data = Vec::new();
    assert_eq!(
        contract.check_inherent(&inherent, 0, 0),
        Err(AccountError::InvalidInherent)
    );
    assert_eq!(
        contract.commit_inherent(&inherent, 0, 0),
        Err(AccountError::InvalidInherent)
    );
}

#[test]
fn it_rejects_invalid_finalize_epoch_inherents() {
    let bls_pair = bls_key_pair();
    let key_pair = ed25519_key_pair();
    let mut contract = make_sample_contract(&ValidatorId::default(), &key_pair, &bls_pair);

    // Invalid inherent
    let mut inherent = Inherent {
        ty: InherentType::FinalizeBatch,
        target: Default::default(),
        value: Coin::from_u64_unchecked(1),
        data: Vec::new(),
    };

    // Invalid value.
    assert_eq!(
        contract.check_inherent(&inherent, 0, 0),
        Err(AccountError::InvalidInherent)
    );
    assert_eq!(
        contract.commit_inherent(&inherent, 0, 0),
        Err(AccountError::InvalidInherent)
    );

    // Invalid data.
    inherent.value = Coin::ZERO;
    inherent.data = vec![1];
    assert_eq!(
        contract.check_inherent(&inherent, 0, 0),
        Err(AccountError::InvalidInherent)
    );
    assert_eq!(
        contract.commit_inherent(&inherent, 0, 0),
        Err(AccountError::InvalidInherent)
    );
}

#[test]
fn it_can_apply_slash_and_finalize_epoch_inherent() {
    let bls_pair = bls_key_pair();
    let key_pair = ed25519_key_pair();
    let validator_id: ValidatorId = [0u8; 20].into();
    let mut contract = make_sample_contract(&ValidatorId::default(), &key_pair, &bls_pair);
    let slot = SlashedSlot {
        slot: 0,
        validator_id: validator_id.clone(),
        event_block: 0,
    };

    // Slash
    let slash = Inherent {
        ty: InherentType::Slash,
        target: Default::default(),
        value: Coin::ZERO,
        data: slot.serialize_to_vec(),
    };
    assert_eq!(contract.check_inherent(&slash, 0, 0), Ok(()));
    assert_eq!(
        contract.commit_inherent(&slash, 0, 0),
        Ok(Some(vec![1, 1, 1]))
    ); // Receipt is boolean set to true.
    assert_eq!(contract.balance, Coin::from_u64_unchecked(300_000_000));
    assert_eq!(contract.active_validators_by_id.len(), 1);
    assert_eq!(contract.inactive_validators_by_id.len(), 0);
    assert_eq!(contract.current_epoch_parking.len(), 1);
    assert_eq!(contract.previous_epoch_parking.len(), 0);
    assert!(contract.current_epoch_parking.contains(&validator_id));

    // Second slash
    assert_eq!(contract.check_inherent(&slash, 0, 0), Ok(()));
    assert_eq!(
        contract.commit_inherent(&slash, 0, 0),
        Ok(Some(vec![0, 0, 0]))
    ); // Receipt is boolean set to false.
    assert_eq!(contract.balance, Coin::from_u64_unchecked(300_000_000));
    assert_eq!(contract.active_validators_by_id.len(), 1);
    assert_eq!(contract.inactive_validators_by_id.len(), 0);
    assert_eq!(contract.current_epoch_parking.len(), 1);
    assert_eq!(contract.previous_epoch_parking.len(), 0);
    assert!(contract.current_epoch_parking.contains(&validator_id));

    // First finalize
    let finalize = Inherent {
        ty: InherentType::FinalizeEpoch,
        target: Default::default(),
        value: Coin::ZERO,
        data: vec![],
    };

    assert_eq!(contract.check_inherent(&finalize, 0, 0), Ok(()));
    assert_eq!(contract.commit_inherent(&finalize, 0, 0), Ok(None));
    assert_eq!(contract.balance, Coin::from_u64_unchecked(300_000_000));
    assert_eq!(contract.active_validators_by_id.len(), 1);
    assert_eq!(contract.inactive_validators_by_id.len(), 0);
    assert_eq!(contract.current_epoch_parking.len(), 0);
    assert_eq!(contract.previous_epoch_parking.len(), 1);
    assert!(contract.previous_epoch_parking.contains(&validator_id));

    // Third slash
    assert_eq!(contract.check_inherent(&slash, 0, 0), Ok(()));
    assert_eq!(
        contract.commit_inherent(&slash, 0, 0),
        Ok(Some(vec![1, 1, 1]))
    );
    assert_eq!(contract.balance, Coin::from_u64_unchecked(300_000_000));
    assert_eq!(contract.active_validators_by_id.len(), 1);
    assert_eq!(contract.inactive_validators_by_id.len(), 0);
    assert_eq!(contract.current_epoch_parking.len(), 1);
    assert_eq!(contract.previous_epoch_parking.len(), 1);
    assert!(contract.current_epoch_parking.contains(&validator_id));
    assert!(contract.previous_epoch_parking.contains(&validator_id));

    // Another finalize
    assert_eq!(contract.check_inherent(&finalize, 0, 0), Ok(()));
    assert_eq!(contract.commit_inherent(&finalize, 0, 0), Ok(None));
    assert_eq!(contract.balance, Coin::from_u64_unchecked(300_000_000));
    assert_eq!(contract.active_validators_by_id.len(), 0);
    assert_eq!(contract.inactive_validators_by_id.len(), 1);
    assert_eq!(contract.current_epoch_parking.len(), 0);
    assert_eq!(contract.previous_epoch_parking.len(), 1);
    assert!(contract.previous_epoch_parking.contains(&validator_id));

    // Another finalize
    assert_eq!(contract.check_inherent(&finalize, 0, 0), Ok(()));
    assert_eq!(contract.commit_inherent(&finalize, 0, 0), Ok(None));
    assert_eq!(contract.balance, Coin::from_u64_unchecked(300_000_000));
    assert_eq!(contract.active_validators_by_id.len(), 0);
    assert_eq!(contract.inactive_validators_by_id.len(), 1);
    assert_eq!(contract.current_epoch_parking.len(), 0);
    assert_eq!(contract.previous_epoch_parking.len(), 0);
}

#[test]
fn it_can_apply_slashes_after_retire() {
    // In this test, we are retiring a validator before it gets slashed and test two scenarios.
    // retire
    //   |
    // slash
    //   |   \
    //   |     \
    // finalize revert slash
    //   |
    // finalize
    let key_pair = ed25519_key_pair();
    let bls_pair = bls_key_pair();
    let validator_id: ValidatorId = [0u8; 20].into();
    let mut contract = make_sample_contract(&validator_id, &key_pair, &bls_pair);
    let slot = SlashedSlot {
        slot: 0,
        validator_id: validator_id.clone(),
        event_block: 0,
    };

    // Check that transaction cannot have any value
    let mut tx_1 = make_signed_incoming_transaction(
        IncomingStakingTransactionData::RetireValidator {
            validator_id: validator_id.clone(),
            signature: Default::default(),
        },
        100,
        &bls_pair,
    );
    assert_eq!(
        tx_1.verify(NetworkId::Dummy),
        Err(TransactionError::InvalidForRecipient)
    );

    // Check that signature is verified
    tx_1.value = Coin::from_u64_unchecked(0);
    assert_eq!(
        contract.commit_incoming_transaction(&tx_1, 2, 0),
        Err(AccountError::InvalidSignature)
    );

    // Retire validator
    let tx_1 = make_signed_incoming_transaction(
        IncomingStakingTransactionData::RetireValidator {
            validator_id: validator_id.clone(),
            signature: Default::default(),
        },
        0,
        &bls_pair,
    );
    assert_eq!(AccountType::verify_incoming_transaction(&tx_1), Ok(()));
    assert_eq!(
        StakingContract::check_incoming_transaction(&tx_1, 2, 0),
        Ok(())
    );
    assert_eq!(contract.commit_incoming_transaction(&tx_1, 2, 0), Ok(None));

    assert_eq!(contract.active_validators_by_id.len(), 0);
    assert_eq!(contract.active_validators_sorted.len(), 0);
    assert_eq!(contract.inactive_validators_by_id.len(), 1);
    assert_eq!(contract.balance, Coin::from_u64_unchecked(300_000_000));

    // Slash
    let slash = Inherent {
        ty: InherentType::Slash,
        target: Default::default(),
        value: Coin::ZERO,
        data: slot.serialize_to_vec(),
    };
    assert_eq!(contract.check_inherent(&slash, 3, 0), Ok(()));
    assert_eq!(
        contract.commit_inherent(&slash, 3, 0),
        Ok(Some(vec![1, 0, 1]))
    ); // Receipt is boolean set to true.
    assert_eq!(contract.current_epoch_parking.len(), 1);
    assert_eq!(contract.previous_epoch_parking.len(), 0);
    assert_eq!(contract.active_validators_sorted.len(), 0);
    assert_eq!(contract.inactive_validators_by_id.len(), 1);
    assert!(contract.current_epoch_parking.contains(&validator_id));

    // Scenario 1: Revert slash
    let mut contract_copy = contract.clone();
    assert_eq!(
        contract_copy.revert_inherent(&slash, 3, 0, Some(&vec![1, 1, 1])),
        Ok(())
    );
    assert_eq!(contract_copy.current_epoch_parking.len(), 0);
    assert_eq!(contract_copy.previous_epoch_parking.len(), 0);
    assert_eq!(contract.active_validators_sorted.len(), 0);
    assert_eq!(contract.inactive_validators_by_id.len(), 1);

    // Scenario 2: First finalize
    let finalize = Inherent {
        ty: InherentType::FinalizeEpoch,
        target: Default::default(),
        value: Coin::ZERO,
        data: vec![],
    };
    assert_eq!(contract.check_inherent(&finalize, 0, 0), Ok(()));
    assert_eq!(contract.commit_inherent(&finalize, 0, 0), Ok(None));
    assert_eq!(contract.current_epoch_parking.len(), 0);
    assert_eq!(contract.previous_epoch_parking.len(), 1);
    assert!(contract.previous_epoch_parking.contains(&validator_id));
    assert_eq!(contract.active_validators_sorted.len(), 0);
    assert_eq!(contract.inactive_validators_by_id.len(), 1);

    // Second finalize
    assert_eq!(contract.check_inherent(&finalize, 0, 0), Ok(()));
    assert_eq!(contract.commit_inherent(&finalize, 0, 0), Ok(None));
    assert_eq!(contract.current_epoch_parking.len(), 0);
    assert_eq!(contract.previous_epoch_parking.len(), 0);
    assert_eq!(contract.active_validators_sorted.len(), 0);
    assert_eq!(contract.inactive_validators_by_id.len(), 1);
}

#[test]
fn it_can_apply_unpark_transactions() {
    let bls_pair = bls_key_pair();
    let key_pair = ed25519_key_pair();
    let validator_id: ValidatorId = [0u8; 20].into();
    let mut contract = make_sample_contract(&ValidatorId::default(), &key_pair, &bls_pair);
    let slot = SlashedSlot {
        slot: 0,
        validator_id: validator_id.clone(),
        event_block: 0,
    };

    // Unpark with invalid value
    let unpark = make_signed_incoming_transaction(
        IncomingStakingTransactionData::UnparkValidator {
            validator_id: validator_id.clone(),
            signature: Default::default(),
        },
        100,
        &bls_pair,
    );
    assert_eq!(
        unpark.verify(NetworkId::Dummy),
        Err(TransactionError::InvalidForRecipient)
    );

    // Unpark with invalid proof
    let priv_key: BlsSecretKey = Deserialize::deserialize(
        &mut &hex::decode(
            "3a12b799f4fa06f1d777695c8ceeeb3fcd831d3342d1f1c7046b9359888bb\
        4e31304d72e819fb685502bad8a7876c7522e4abdf1bd2f4a55306164029e74922a4dd85895b9e54d45139d2204\
        1e83f477eb8b489c19a41aa0529b8add804d0000",
        )
        .unwrap()[..],
    )
    .unwrap();
    let bls_pair2: BlsKeyPair = priv_key.into();
    let validator_id2: ValidatorId = [2u8; 20].into();
    let unpark = make_signed_incoming_transaction(
        IncomingStakingTransactionData::UnparkValidator {
            validator_id: validator_id.clone(),
            signature: Default::default(),
        },
        0,
        &bls_pair2,
    );
    assert_eq!(
        contract.commit_incoming_transaction(&unpark, 2, 0),
        Err(AccountError::InvalidSignature)
    );

    // Invalid type
    let mut unpark = make_signed_incoming_transaction(
        IncomingStakingTransactionData::UnparkValidator {
            validator_id: validator_id.clone(),
            signature: Default::default(),
        },
        0,
        &bls_pair,
    );
    unpark.data = Vec::new();
    if let Err(TransactionError::InvalidSerialization(_e)) =
        AccountType::verify_incoming_transaction(&unpark)
    {
        // Ok
    } else {
        assert!(false, "Transaction should have been rejected.");
    }

    // Unpark with address that is not staked
    let unpark = make_signed_incoming_transaction(
        IncomingStakingTransactionData::UnparkValidator {
            validator_id: validator_id2,
            signature: Default::default(),
        },
        0,
        &bls_pair2,
    );
    assert_eq!(unpark.verify(NetworkId::Dummy), Ok(()));
    assert_eq!(
        StakingContract::check_incoming_transaction(&unpark, 2, 0),
        Ok(())
    );
    assert_eq!(
        contract.commit_incoming_transaction(&unpark, 2, 0),
        Err(AccountError::InvalidForRecipient)
    );

    // Unpark with address that is not parked
    let unpark = make_signed_incoming_transaction(
        IncomingStakingTransactionData::UnparkValidator {
            validator_id,
            signature: Default::default(),
        },
        0,
        &bls_pair,
    );
    assert_eq!(unpark.verify(NetworkId::Dummy), Ok(()));
    assert_eq!(
        StakingContract::check_incoming_transaction(&unpark, 2, 0),
        Ok(())
    );
    assert_eq!(
        contract.commit_incoming_transaction(&unpark, 2, 0),
        Err(AccountError::InvalidForRecipient)
    );

    // Slash
    let slash = Inherent {
        ty: InherentType::Slash,
        target: Default::default(),
        value: Coin::ZERO,
        data: slot.serialize_to_vec(),
    };
    assert_eq!(contract.check_inherent(&slash, 0, 0), Ok(()));
    assert_eq!(
        contract.commit_inherent(&slash, 0, 0),
        Ok(Some(vec![1, 1, 1]))
    ); // Receipt is boolean set to true.

    // Unpark
    let mut contract_copy = contract.clone();
    assert_eq!(
        StakingContract::check_incoming_transaction(&unpark, 2, 0),
        Ok(())
    );
    assert!(contract_copy
        .commit_incoming_transaction(&unpark, 2, 0)
        .is_ok());
    assert_eq!(contract_copy.current_epoch_parking.len(), 0);
    assert_eq!(contract_copy.previous_epoch_parking.len(), 0);
    assert_eq!(contract.balance, Coin::from_u64_unchecked(300_000_000));

    // Build on previous contract state and finalize and slash
    let finalize = Inherent {
        ty: InherentType::FinalizeEpoch,
        target: Default::default(),
        value: Coin::ZERO,
        data: vec![],
    };
    assert_eq!(contract.check_inherent(&finalize, 0, 0), Ok(()));
    assert_eq!(contract.commit_inherent(&finalize, 0, 0), Ok(None));
    assert_eq!(contract.check_inherent(&slash, 0, 0), Ok(()));
    assert_eq!(
        contract.commit_inherent(&slash, 0, 0),
        Ok(Some(vec![1, 1, 1]))
    );
    assert_eq!(contract.current_epoch_parking.len(), 1);
    assert_eq!(contract.previous_epoch_parking.len(), 1);
    assert_eq!(contract.balance, Coin::from_u64_unchecked(300_000_000));

    // Unpark
    assert_eq!(
        StakingContract::check_incoming_transaction(&unpark, 2, 0),
        Ok(())
    );
    assert!(contract.commit_incoming_transaction(&unpark, 2, 0).is_ok());
    assert_eq!(contract.current_epoch_parking.len(), 0);
    assert_eq!(contract.previous_epoch_parking.len(), 0);
    assert_eq!(contract.balance, Coin::from_u64_unchecked(300_000_000));
}

#[test]
fn it_can_revert_unpark_transactions() {
    let bls_pair = bls_key_pair();
    let key_pair = ed25519_key_pair();
    let validator_id: ValidatorId = [0u8; 20].into();
    let contract = make_sample_contract(&validator_id, &key_pair, &bls_pair);
    let slot = SlashedSlot {
        slot: 0,
        validator_id: validator_id.clone(),
        event_block: 0,
    };

    let unpark = make_signed_incoming_transaction(
        IncomingStakingTransactionData::UnparkValidator {
            validator_id: validator_id.clone(),
            signature: Default::default(),
        },
        0,
        &bls_pair,
    );

    // Slash
    let mut parked_in_current = contract;
    let slash = Inherent {
        ty: InherentType::Slash,
        target: Default::default(),
        value: Coin::ZERO,
        data: slot.serialize_to_vec(),
    };
    assert_eq!(parked_in_current.check_inherent(&slash, 0, 0), Ok(()));
    assert_eq!(
        parked_in_current.commit_inherent(&slash, 0, 0),
        Ok(Some(vec![1, 1, 1]))
    ); // Receipt is boolean set to true.

    // Unpark
    let incoming_receipt = parked_in_current.commit_incoming_transaction(&unpark, 2, 0);
    assert!(incoming_receipt.is_ok());
    assert_eq!(parked_in_current.current_epoch_parking.len(), 0);
    assert_eq!(parked_in_current.previous_epoch_parking.len(), 0);

    // Revert unpark
    assert_eq!(
        parked_in_current.revert_incoming_transaction(
            &unpark,
            2,
            0,
            incoming_receipt.unwrap().as_ref()
        ),
        Ok(())
    );
    assert_eq!(parked_in_current.current_epoch_parking.len(), 1);
    assert!(parked_in_current
        .current_epoch_parking
        .contains(&validator_id));
    assert_eq!(parked_in_current.previous_epoch_parking.len(), 0);

    // Park, unpark and revert unpark in previous epoch
    let mut parked_in_previous = parked_in_current.clone();
    let finalize = Inherent {
        ty: InherentType::FinalizeEpoch,
        target: Default::default(),
        value: Coin::ZERO,
        data: vec![],
    };
    assert_eq!(parked_in_previous.check_inherent(&finalize, 0, 0), Ok(()));
    assert_eq!(
        parked_in_previous.commit_inherent(&finalize, 0, 0),
        Ok(None)
    );
    assert_eq!(parked_in_previous.current_epoch_parking.len(), 0);
    assert_eq!(parked_in_previous.previous_epoch_parking.len(), 1);
    assert!(parked_in_previous
        .previous_epoch_parking
        .contains(&validator_id));

    // Unpark
    let incoming_receipt = parked_in_previous.commit_incoming_transaction(&unpark, 2, 0);
    assert!(incoming_receipt.is_ok());
    assert_eq!(parked_in_previous.current_epoch_parking.len(), 0);
    assert_eq!(parked_in_previous.previous_epoch_parking.len(), 0);

    // Revert unpark
    assert_eq!(
        parked_in_previous.revert_incoming_transaction(
            &unpark,
            2,
            0,
            incoming_receipt.unwrap().as_ref()
        ),
        Ok(())
    );
    assert_eq!(parked_in_previous.current_epoch_parking.len(), 0);
    assert_eq!(parked_in_previous.previous_epoch_parking.len(), 1);
    assert!(parked_in_previous
        .previous_epoch_parking
        .contains(&validator_id));

    // Park, unpark and revert unpark in both epochs
    let mut parked_in_both = parked_in_current;
    let finalize = Inherent {
        ty: InherentType::FinalizeEpoch,
        target: Default::default(),
        value: Coin::ZERO,
        data: vec![],
    };
    assert_eq!(parked_in_both.check_inherent(&finalize, 0, 0), Ok(()));
    assert_eq!(parked_in_both.commit_inherent(&finalize, 0, 0), Ok(None));
    assert_eq!(parked_in_both.check_inherent(&slash, 0, 0), Ok(()));
    assert_eq!(
        parked_in_both.commit_inherent(&slash, 0, 0),
        Ok(Some(vec![1, 1, 1]))
    );
    assert_eq!(parked_in_both.current_epoch_parking.len(), 1);
    assert!(parked_in_both.current_epoch_parking.contains(&validator_id));
    assert_eq!(parked_in_both.previous_epoch_parking.len(), 1);
    assert!(parked_in_both
        .previous_epoch_parking
        .contains(&validator_id));

    // Unpark
    let incoming_receipt = parked_in_both.commit_incoming_transaction(&unpark, 2, 0);
    assert!(incoming_receipt.is_ok());
    assert_eq!(parked_in_both.current_epoch_parking.len(), 0);
    assert_eq!(parked_in_both.previous_epoch_parking.len(), 0);

    // Revert unpark
    assert_eq!(
        parked_in_both.revert_incoming_transaction(
            &unpark,
            2,
            0,
            incoming_receipt.unwrap().as_ref()
        ),
        Ok(())
    );
    assert_eq!(parked_in_both.current_epoch_parking.len(), 1);
    assert!(parked_in_both.current_epoch_parking.contains(&validator_id));
    assert_eq!(parked_in_both.previous_epoch_parking.len(), 1);
    assert!(parked_in_both
        .previous_epoch_parking
        .contains(&validator_id));
}

#[test]
fn it_will_never_revert_finalized_epoch_inherents() {
    let bls_pair = bls_key_pair();
    let key_pair = ed25519_key_pair();
    let mut contract = make_sample_contract(&ValidatorId::default(), &key_pair, &bls_pair);

    let finalize = Inherent {
        ty: InherentType::FinalizeBatch,
        target: Default::default(),
        value: Coin::ZERO,
        data: vec![],
    };

    assert_eq!(contract.check_inherent(&finalize, 0, 0), Ok(()));
    assert_eq!(contract.commit_inherent(&finalize, 0, 0), Ok(None));
    assert_eq!(
        contract.revert_inherent(&finalize, 0, 0, None),
        Err(AccountError::InvalidForTarget)
    );
}

#[test]
fn it_can_revert_slash_inherent() {
    let bls_pair = bls_key_pair();
    let key_pair = ed25519_key_pair();
    let validator_id: ValidatorId = [0u8; 20].into();
    let mut contract = make_sample_contract(&ValidatorId::default(), &key_pair, &bls_pair);
    let slot = SlashedSlot {
        slot: 0,
        validator_id: validator_id.clone(),
        event_block: 0,
    };

    // Slash
    let slash = Inherent {
        ty: InherentType::Slash,
        target: Default::default(),
        value: Coin::ZERO,
        data: slot.serialize_to_vec(),
    };
    assert_eq!(contract.check_inherent(&slash, 0, 0), Ok(()));
    assert_eq!(
        contract.commit_inherent(&slash, 0, 0),
        Ok(Some(vec![1, 1, 1]))
    ); // Receipt is boolean set to true.
    assert_eq!(contract.current_epoch_parking.len(), 1);
    assert_eq!(contract.previous_epoch_parking.len(), 0);
    assert!(contract.current_epoch_parking.contains(&validator_id));
    assert_eq!(contract.balance, Coin::from_u64_unchecked(300_000_000));

    // Revert slash
    let mut contract_copy = contract.clone();
    assert_eq!(
        contract_copy.revert_inherent(&slash, 0, 0, Some(&vec![1, 1, 1])),
        Ok(())
    );
    assert_eq!(contract_copy.current_epoch_parking.len(), 0);
    assert_eq!(contract_copy.previous_epoch_parking.len(), 0);
    assert_eq!(contract.balance, Coin::from_u64_unchecked(300_000_000));

    // First finalize
    let finalize = Inherent {
        ty: InherentType::FinalizeEpoch,
        target: Default::default(),
        value: Coin::ZERO,
        data: vec![],
    };
    assert_eq!(contract.check_inherent(&finalize, 0, 0), Ok(()));
    assert_eq!(contract.commit_inherent(&finalize, 0, 0), Ok(None));
    assert_eq!(contract.current_epoch_parking.len(), 0);
    assert_eq!(contract.previous_epoch_parking.len(), 1);
    assert!(contract.previous_epoch_parking.contains(&validator_id));
    assert_eq!(contract.balance, Coin::from_u64_unchecked(300_000_000));

    // Revert slash after finalize is impossible
    let mut contract_copy = contract.clone();
    assert_eq!(
        contract_copy.revert_inherent(&slash, 0, 0, Some(&vec![1, 1, 1])),
        Err(AccountError::InvalidInherent)
    );

    // Slash multiple times and revert one of the slashes
    // This should *not* remove the slash
    // Slash 1
    assert_eq!(contract.check_inherent(&slash, 0, 0), Ok(()));
    assert_eq!(
        contract.commit_inherent(&slash, 0, 0),
        Ok(Some(vec![1, 1, 1]))
    ); // Receipt is boolean set to true.
    assert_eq!(contract.current_epoch_parking.len(), 1);
    assert_eq!(contract.previous_epoch_parking.len(), 1);
    assert!(contract.current_epoch_parking.contains(&validator_id));
    assert!(contract.previous_epoch_parking.contains(&validator_id));
    assert_eq!(contract.balance, Coin::from_u64_unchecked(300_000_000));

    // Slash 2
    assert_eq!(contract.check_inherent(&slash, 0, 0), Ok(()));
    assert_eq!(
        contract.commit_inherent(&slash, 0, 0),
        Ok(Some(vec![0, 0, 0]))
    ); // Receipt is boolean set to false.
    assert_eq!(contract.current_epoch_parking.len(), 1);
    assert_eq!(contract.previous_epoch_parking.len(), 1);
    assert!(contract.current_epoch_parking.contains(&validator_id));
    assert!(contract.previous_epoch_parking.contains(&validator_id));
    assert_eq!(contract.balance, Coin::from_u64_unchecked(300_000_000));

    // Revert second slash
    assert_eq!(
        contract.revert_inherent(&slash, 0, 0, Some(&vec![0, 0, 0])),
        Ok(())
    );
    assert_eq!(contract.current_epoch_parking.len(), 1);
    assert_eq!(contract.previous_epoch_parking.len(), 1);
    assert!(contract.current_epoch_parking.contains(&validator_id));
    assert!(contract.previous_epoch_parking.contains(&validator_id));
    assert_eq!(contract.balance, Coin::from_u64_unchecked(300_000_000));

    // Revert first slash
    assert_eq!(
        contract.revert_inherent(&slash, 0, 0, Some(&vec![1, 1, 1])),
        Ok(())
    );
    assert_eq!(contract.current_epoch_parking.len(), 0);
    assert_eq!(contract.previous_epoch_parking.len(), 1);
    assert!(contract.previous_epoch_parking.contains(&validator_id));
    assert_eq!(contract.balance, Coin::from_u64_unchecked(300_000_000));
}

#[test]
fn it_can_build_a_validator_set() {
    let bls_key1 = bls_key_pair();
    let validator_key1 = bls_key1.public_key.compress();
    let validator_id1: ValidatorId = [1; 20].into();
    let bls_key2 = BlsKeyPair::from(
        BlsSecretKey::deserialize_from_vec(
            &hex::decode(
                "ad544a120adee180dd638876bd423b984e3ba6bad8af0006beb72da1db1f617c371195340c896ffca6e2a6ef7205df012f7586107b58c454fffdecba1d2186804b965213f8c1a671868db31820f698aa5893102b2970da04b6be8b4d2ab50100",
            )
            .unwrap(),
        )
        .unwrap(),
    );
    let validator_key2 = bls_key2.public_key.compress();
    let validator_id2: ValidatorId = [2; 20].into();
    let bls_key3 = BlsKeyPair::from(
        BlsSecretKey::deserialize_from_vec(
            &hex::decode(
                "f61bd4751bf4a480b6ccf66cf0ac8d707cb5afaad5d26eca496bbc9762b9dc8cc2efaab2d0abf94edf2d8d3211113dd4ab487fb042697a8e515c62f76a2119f7af3bb247d01cea7f699d60d09fe50eb4eda6b33313e88c3929c397c3dcaa0000",
            )
            .unwrap(),
        )
        .unwrap(),
    );
    let validator_key3 = bls_key3.public_key.compress();
    let validator_id3: ValidatorId = [3; 20].into();
    let staker1 = Address::from_any_str("3b4fe0cd29f89011282e7d9d2f4917fadfe90586").unwrap();
    let staker2 = Address::from_any_str("59ed95062ce9322fe66d102f9cde1aadba76a022").unwrap();
    let staker3 = Address::from_any_str("adbfca612387ffab95acfa8a1a1657e5f9b9e4c2").unwrap();

    // Create arbitrary BLS signature as seed
    let seed_vec = hex::decode(
        "01737df5f635123d5c8d74d3ed56a0f3ddbda0418b6b3369bd0904d03b549d03ab4f71008ebbd0ff721203766a63352246ad22c1586a973b690ade6330704175d4188bdf7862182d421ca90b5dd8822cb7b98444ae2d7537533c32f2191a8700",
    )
    .unwrap();
    let seed = BlsSignature::deserialize_from_vec(&seed_vec).unwrap();

    // Fill contract with one validator without stakes
    let mut contract = make_empty_contract();
    contract
        .create_validator(
            validator_id1.clone(),
            validator_key1.clone(),
            staker1.clone(),
            Coin::from_u64_unchecked(100_000_000),
        )
        .unwrap();

    let slots = contract.select_validators(&seed.compress().into());
    assert_eq!(slots.num_validators(), 1);
    assert_eq!(
        slots.get_validator(0).public_key.compressed(),
        &validator_key1
    );

    // Fill contract with stakes
    contract
        .stake(
            staker1.clone(),
            Coin::from_u64_unchecked(100_000_000),
            &validator_id1,
        )
        .unwrap();
    contract
        .stake(
            staker2.clone(),
            Coin::from_u64_unchecked(100_000_000),
            &validator_id1,
        )
        .unwrap();

    let slots = contract.select_validators(&seed.compress().into());
    assert_eq!(slots.num_validators(), 1);
    assert_eq!(
        slots.get_validator(0).public_key.compressed(),
        &validator_key1
    );

    // Add more validators and stakes
    contract
        .create_validator(
            validator_id2.clone(),
            validator_key2.clone(),
            staker2.clone(),
            Coin::from_u64_unchecked(100_000_000),
        )
        .unwrap();
    contract
        .create_validator(
            validator_id3.clone(),
            validator_key3.clone(),
            staker3.clone(),
            Coin::from_u64_unchecked(100_000_000),
        )
        .unwrap();

    contract
        .stake(
            staker2,
            Coin::from_u64_unchecked(100_000_000),
            &validator_id2,
        )
        .unwrap();
    contract
        .stake(
            staker3.clone(),
            Coin::from_u64_unchecked(100_000_000),
            &validator_id2,
        )
        .unwrap();
    contract
        .stake(
            staker1,
            Coin::from_u64_unchecked(100_000_000),
            &validator_id3,
        )
        .unwrap();
    contract
        .stake(
            staker3,
            Coin::from_u64_unchecked(100_000_000),
            &validator_id3,
        )
        .unwrap();

    // Check balances
    assert_eq!(
        contract.get_validator(&validator_id1).unwrap().balance,
        Coin::from_u64_unchecked(300_000_000)
    );
    assert_eq!(
        contract.get_validator(&validator_id2).unwrap().balance,
        Coin::from_u64_unchecked(300_000_000)
    );
    assert_eq!(
        contract.get_validator(&validator_id3).unwrap().balance,
        Coin::from_u64_unchecked(300_000_000)
    );

    // Test potential validator selection by stake
    let slots = contract.select_validators(&seed.compress().into());
    assert_eq!(slots.num_validators(), 3);
    assert_eq!(
        slots.get_validator(0).public_key.compressed(),
        &validator_key1
    );
    assert_eq!(
        slots.get_validator(200).public_key.compressed(),
        &validator_key2
    );
    assert_eq!(
        slots.get_validator(500).public_key.compressed(),
        &validator_key3
    );

    assert_eq!(slots.get_validator(0).validator_id, validator_id1);
    assert_eq!(slots.get_validator(200).validator_id, validator_id2);
    assert_eq!(slots.get_validator(500).validator_id, validator_id3);

    // TODO More tests
}

#[test]
fn it_can_apply_validator_signalling() {
    let bls_pair = bls_key_pair();
    let key_pair = ed25519_key_pair();
    let mut contract = make_empty_contract();
    let validator_key = bls_pair.public_key.compress();
    let bls_pair2 = BlsKeyPair::from(
        BlsSecretKey::deserialize_from_vec(
            &hex::decode(
                "c37da6085717a01d58f86a091579a625675fdc9cecb6e38404c0c21e8b0b570c38\
            b0c414d52bd80a2594497e6d51b9a58c2e8d21976c4482ebbee8dcf7b92f7c1988eb075d3c3f5ecad823d6c\
            37abfd02c20c467493d544a8e1ab1dd367e0100",
            )
            .unwrap(),
        )
        .unwrap(),
    );
    let validator_key2 = bls_pair2.public_key.compress();

    // This test is supposed to test all validator signalling actions.
    // To this end, we:
    // 1. Set up a validator.
    // 1a. Update a validator.
    // 2. Retire a validator.
    // 2a. Update a validator.
    // 3. Re-activate a validator.
    // 4. Drop a validator.
    // Unparking is already well tested, so we left it out here.
    // All actions are applied and reverted for test purposes.

    // 1. Create first validator.
    let tx = make_incoming_transaction(
        IncomingStakingTransactionData::CreateValidator {
            validator_key: validator_key.clone(),
            proof_of_knowledge: bls_pair.sign(&validator_key.serialize_to_vec()).compress(),
            reward_address: Address::from([3u8; 20]),
        },
        100_000_000,
    );
    let validator_id: ValidatorId = tx.hash::<Blake2bHash>().as_slice()[0..20].into();

    assert_eq!(AccountType::verify_incoming_transaction(&tx), Ok(()));
    assert_eq!(
        StakingContract::check_incoming_transaction(&tx, 2, 0),
        Ok(())
    );
    assert_eq!(contract.commit_incoming_transaction(&tx, 2, 0), Ok(None));

    // Verify contract.
    assert_eq!(contract.balance, Coin::from_u64_unchecked(100_000_000));
    assert_eq!(contract.active_validators_sorted.len(), 1);
    assert_eq!(contract.active_validators_by_id.len(), 1);
    assert_eq!(contract.inactive_validators_by_id.len(), 0);
    assert_eq!(contract.inactive_stake_by_address.len(), 0);
    let validator = contract.get_validator(&validator_id).unwrap();
    assert_eq!(validator.balance, Coin::from_u64_unchecked(100_000_000));
    assert_eq!(validator.active_stake_by_address.read().len(), 0);
    assert_eq!(&validator.validator_key, &validator_key);
    assert_eq!(validator.reward_address, Address::from([3u8; 20]));

    // Check revert.
    let mut contract_copy = contract.clone();
    assert_eq!(
        contract_copy.revert_incoming_transaction(&tx, 2, 0, None),
        Ok(())
    );

    // Verify contract.
    assert_eq!(contract_copy.balance, Coin::from_u64_unchecked(0));
    assert_eq!(contract_copy.active_validators_sorted.len(), 0);
    assert_eq!(contract_copy.active_validators_by_id.len(), 0);
    assert_eq!(contract_copy.inactive_validators_by_id.len(), 0);
    assert_eq!(contract_copy.inactive_stake_by_address.len(), 0);

    // Verify that no two validators with the same id can exist.
    assert_eq!(
        contract.commit_incoming_transaction(&tx, 2, 0),
        Err(AccountError::InvalidForRecipient)
    );

    // Create stake for the validator.
    contract
        .stake(
            Address::from(&key_pair),
            Coin::from_u64_unchecked(100_000_000),
            &validator_id,
        )
        .unwrap();

    // 1a. Update the validator.
    let mut contract_copy = contract.clone();

    // Incorrect proof of knowledge.
    let tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::UpdateValidator {
            validator_id: validator_id.clone(),
            old_validator_key: validator_key.clone(),
            new_validator_key: Some(validator_key2.clone()),
            new_proof_of_knowledge: Some(
                bls_pair2.sign(&validator_key.serialize_to_vec()).compress(),
            ),
            signature: Default::default(),
            new_reward_address: None,
        },
        0,
        &bls_pair,
    );
    assert_eq!(
        AccountType::verify_incoming_transaction(&tx),
        Err(TransactionError::InvalidData)
    );

    // Wrong validator id
    let tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::UpdateValidator {
            validator_id: validator_id.clone(),
            old_validator_key: validator_key2.clone(),
            new_validator_key: Some(validator_key2.clone()),
            new_proof_of_knowledge: Some(
                bls_pair2
                    .sign(&validator_key2.serialize_to_vec())
                    .compress(),
            ),
            signature: Default::default(),
            new_reward_address: None,
        },
        0,
        &bls_pair2,
    );
    assert_eq!(
        contract_copy.commit_incoming_transaction(&tx, 3, 0),
        Err(AccountError::InvalidSignature)
    );

    // Correct update
    let tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::UpdateValidator {
            validator_id: validator_id.clone(),
            old_validator_key: validator_key.clone(),
            new_validator_key: Some(validator_key2.clone()),
            new_proof_of_knowledge: Some(
                bls_pair2
                    .sign(&validator_key2.serialize_to_vec())
                    .compress(),
            ),
            signature: Default::default(),
            new_reward_address: None,
        },
        0,
        &bls_pair,
    );

    assert_eq!(AccountType::verify_incoming_transaction(&tx), Ok(()));
    assert_eq!(
        StakingContract::check_incoming_transaction(&tx, 3, 0),
        Ok(())
    );
    let receipt = contract_copy
        .commit_incoming_transaction(&tx, 3, 0)
        .unwrap();
    assert!(receipt.is_some());

    // Verify contract.
    assert_eq!(contract_copy.balance, Coin::from_u64_unchecked(200_000_000));
    assert_eq!(contract_copy.active_validators_sorted.len(), 1);
    assert_eq!(contract_copy.active_validators_by_id.len(), 1);
    assert_eq!(contract_copy.inactive_validators_by_id.len(), 0);
    assert_eq!(contract_copy.inactive_stake_by_address.len(), 0);
    // The validator key stays the same after updating
    let validator = contract_copy.get_validator(&validator_id).unwrap();
    assert_eq!(validator.balance, Coin::from_u64_unchecked(200_000_000));
    assert_eq!(validator.active_stake_by_address.read().len(), 1);
    assert_eq!(&validator.validator_key, &validator_key2);
    assert_eq!(validator.reward_address, Address::from([3u8; 20]));

    // Revert update.
    assert_eq!(
        contract_copy.revert_incoming_transaction(&tx, 3, 0, receipt.as_ref()),
        Ok(())
    );

    // Verify contract.
    assert_eq!(contract_copy.balance, Coin::from_u64_unchecked(200_000_000));
    assert_eq!(contract_copy.active_validators_sorted.len(), 1);
    assert_eq!(contract_copy.active_validators_by_id.len(), 1);
    assert_eq!(contract_copy.inactive_validators_by_id.len(), 0);
    assert_eq!(contract_copy.inactive_stake_by_address.len(), 0);
    let validator = contract_copy.get_validator(&validator_id).unwrap();
    assert_eq!(validator.balance, Coin::from_u64_unchecked(200_000_000));
    assert_eq!(validator.active_stake_by_address.read().len(), 1);
    assert_eq!(&validator.validator_key, &validator_key);
    assert_eq!(validator.reward_address, Address::from([3u8; 20]));

    // 2. Retire the validator.
    let tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::RetireValidator {
            validator_id: validator_id.clone(),
            signature: Default::default(),
        },
        0,
        &bls_pair,
    );
    assert_eq!(AccountType::verify_incoming_transaction(&tx), Ok(()));
    assert_eq!(
        StakingContract::check_incoming_transaction(&tx, 3, 0),
        Ok(())
    );
    assert_eq!(contract.commit_incoming_transaction(&tx, 3, 0), Ok(None));

    // Verify contract.
    assert_eq!(contract.balance, Coin::from_u64_unchecked(200_000_000));
    assert_eq!(contract.active_validators_sorted.len(), 0);
    assert_eq!(contract.active_validators_by_id.len(), 0);
    assert_eq!(contract.inactive_validators_by_id.len(), 1);
    assert_eq!(contract.inactive_stake_by_address.len(), 0);
    let validator = contract.get_validator(&validator_id).unwrap();
    assert_eq!(validator.balance, Coin::from_u64_unchecked(200_000_000));
    assert_eq!(validator.active_stake_by_address.read().len(), 1);
    assert_eq!(&validator.validator_key, &validator_key);
    assert_eq!(validator.reward_address, Address::from([3u8; 20]));
    let inactive_validator = contract
        .inactive_validators_by_id
        .get(&validator_id)
        .unwrap();
    assert_eq!(inactive_validator.retire_time, 3);

    // Check revert.
    let mut contract_copy = contract.clone();
    assert_eq!(
        contract_copy.revert_incoming_transaction(&tx, 2, 0, None),
        Ok(())
    );

    // Verify contract.
    assert_eq!(contract_copy.balance, Coin::from_u64_unchecked(200_000_000));
    assert_eq!(contract_copy.active_validators_sorted.len(), 1);
    assert_eq!(contract_copy.active_validators_by_id.len(), 1);
    assert_eq!(contract_copy.inactive_validators_by_id.len(), 0);
    assert_eq!(contract_copy.inactive_stake_by_address.len(), 0);
    let validator = contract.get_validator(&validator_id).unwrap();
    assert_eq!(validator.balance, Coin::from_u64_unchecked(200_000_000));
    assert_eq!(validator.active_stake_by_address.read().len(), 1);
    assert_eq!(&validator.validator_key, &validator_key);
    assert_eq!(validator.reward_address, Address::from([3u8; 20]));

    // 2a. Update the validator.
    let mut contract_copy = contract.clone();
    let tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::UpdateValidator {
            validator_id: validator_id.clone(),
            old_validator_key: validator_key.clone(),
            new_validator_key: None,
            new_proof_of_knowledge: None,
            signature: Default::default(),
            new_reward_address: Some(Address::from([4u8; 20])),
        },
        0,
        &bls_pair,
    );
    assert_eq!(AccountType::verify_incoming_transaction(&tx), Ok(()));
    assert_eq!(
        StakingContract::check_incoming_transaction(&tx, 3, 0),
        Ok(())
    );
    let receipt = contract_copy
        .commit_incoming_transaction(&tx, 3, 0)
        .unwrap();
    assert!(receipt.is_some());

    // Verify contract.
    assert_eq!(contract_copy.balance, Coin::from_u64_unchecked(200_000_000));
    assert_eq!(contract_copy.active_validators_sorted.len(), 0);
    assert_eq!(contract_copy.active_validators_by_id.len(), 0);
    assert_eq!(contract_copy.inactive_validators_by_id.len(), 1);
    assert_eq!(contract_copy.inactive_stake_by_address.len(), 0);
    let validator = contract_copy.get_validator(&validator_id).unwrap();
    assert_eq!(validator.balance, Coin::from_u64_unchecked(200_000_000));
    assert_eq!(validator.active_stake_by_address.read().len(), 1);
    assert_eq!(&validator.validator_key, &validator_key);
    assert_eq!(validator.reward_address, Address::from([4u8; 20]));

    // Revert update.
    assert_eq!(
        contract_copy.revert_incoming_transaction(&tx, 3, 0, receipt.as_ref()),
        Ok(())
    );

    // Verify contract.
    assert_eq!(contract_copy.balance, Coin::from_u64_unchecked(200_000_000));
    assert_eq!(contract_copy.active_validators_sorted.len(), 0);
    assert_eq!(contract_copy.active_validators_by_id.len(), 0);
    assert_eq!(contract_copy.inactive_validators_by_id.len(), 1);
    assert_eq!(contract_copy.inactive_stake_by_address.len(), 0);
    let validator = contract_copy.get_validator(&validator_id).unwrap();
    assert_eq!(validator.balance, Coin::from_u64_unchecked(200_000_000));
    assert_eq!(validator.active_stake_by_address.read().len(), 1);
    assert_eq!(&validator.validator_key, &validator_key);
    assert_eq!(validator.reward_address, Address::from([3u8; 20]));

    // 3. Re-activate the validator.
    let mut contract_copy = contract.clone();
    let tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::ReactivateValidator {
            validator_id: validator_id.clone(),
            signature: Default::default(),
        },
        0,
        &bls_pair,
    );
    assert_eq!(AccountType::verify_incoming_transaction(&tx), Ok(()));
    assert_eq!(
        StakingContract::check_incoming_transaction(&tx, 4, 0),
        Ok(())
    );
    let receipt = contract_copy
        .commit_incoming_transaction(&tx, 4, 0)
        .unwrap();
    assert!(receipt.is_some());

    // Verify contract.
    assert_eq!(contract_copy.balance, Coin::from_u64_unchecked(200_000_000));
    assert_eq!(contract_copy.active_validators_sorted.len(), 1);
    assert_eq!(contract_copy.active_validators_by_id.len(), 1);
    assert_eq!(contract_copy.inactive_validators_by_id.len(), 0);
    assert_eq!(contract_copy.inactive_stake_by_address.len(), 0);
    let validator = contract_copy.get_validator(&validator_id).unwrap();
    assert_eq!(validator.balance, Coin::from_u64_unchecked(200_000_000));
    assert_eq!(validator.active_stake_by_address.read().len(), 1);
    assert_eq!(&validator.validator_key, &validator_key);
    assert_eq!(validator.reward_address, Address::from([3u8; 20]));

    // Check revert.
    let mut reactivated_contract = contract_copy.clone();
    assert_eq!(
        contract_copy.revert_incoming_transaction(&tx, 2, 0, receipt.as_ref()),
        Ok(())
    );

    // Verify contract.
    assert_eq!(contract_copy.balance, Coin::from_u64_unchecked(200_000_000));
    assert_eq!(contract_copy.active_validators_sorted.len(), 0);
    assert_eq!(contract_copy.active_validators_by_id.len(), 0);
    assert_eq!(contract_copy.inactive_validators_by_id.len(), 1);
    assert_eq!(contract_copy.inactive_stake_by_address.len(), 0);
    let validator = contract_copy.get_validator(&validator_id).unwrap();
    assert_eq!(validator.balance, Coin::from_u64_unchecked(200_000_000));
    assert_eq!(validator.active_stake_by_address.read().len(), 1);
    assert_eq!(&validator.validator_key, &validator_key);
    assert_eq!(validator.reward_address, Address::from([3u8; 20]));
    let inactive_validator = contract_copy
        .inactive_validators_by_id
        .get(&validator_id)
        .unwrap();
    assert_eq!(inactive_validator.retire_time, 3);

    // 4. Drop a validator.
    // Check that active validators cannot be dropped.
    let tx = make_drop_transaction(validator_id.clone(), &bls_pair, 99_999_900);
    assert_eq!(AccountType::verify_outgoing_transaction(&tx), Ok(()));
    assert_eq!(
        reactivated_contract.check_outgoing_transaction(&tx, 3000, 0),
        Err(AccountError::InvalidForSender)
    );
    assert_eq!(
        reactivated_contract.commit_outgoing_transaction(&tx, 3000, 0),
        Err(AccountError::InvalidForSender)
    );

    // Invalid values.
    // a) zero value
    let tx = make_drop_transaction(validator_id.clone(), &bls_pair, 0);
    assert_eq!(
        AccountType::verify_outgoing_transaction(&tx),
        Err(TransactionError::ZeroValue)
    );

    // b) too high value
    let tx = make_drop_transaction(validator_id.clone(), &bls_pair, 200_000_000);
    assert_eq!(AccountType::verify_outgoing_transaction(&tx), Ok(()));
    assert_eq!(
        contract.check_outgoing_transaction(&tx, 3000, 0),
        Err(AccountError::InsufficientFunds {
            needed: Coin::from_u64_unchecked(300_000_100),
            balance: Coin::from_u64_unchecked(200_000_000),
        })
    );
    assert_eq!(
        contract.commit_outgoing_transaction(&tx, 3000, 0),
        Err(AccountError::InsufficientFunds {
            needed: Coin::from_u64_unchecked(300_000_100),
            balance: Coin::from_u64_unchecked(200_000_000),
        })
    );

    // c) too low value
    let tx = make_drop_transaction(validator_id.clone(), &bls_pair, 90_000_000);
    assert_eq!(AccountType::verify_outgoing_transaction(&tx), Ok(()));
    assert_eq!(
        contract.check_outgoing_transaction(&tx, 3000, 0),
        Err(AccountError::InsufficientFunds {
            needed: Coin::from_u64_unchecked(190_000_100),
            balance: Coin::from_u64_unchecked(200_000_000),
        })
    );
    assert_eq!(
        contract.commit_outgoing_transaction(&tx, 3000, 0),
        Err(AccountError::InsufficientFunds {
            needed: Coin::from_u64_unchecked(190_000_100),
            balance: Coin::from_u64_unchecked(200_000_000),
        })
    );

    // Invalid timing.
    let tx = make_drop_transaction(validator_id.clone(), &bls_pair, 99_999_900);
    assert_eq!(AccountType::verify_outgoing_transaction(&tx), Ok(()));
    assert_eq!(
        contract.check_outgoing_transaction(&tx, 4, 0),
        Err(AccountError::InvalidForSender)
    );
    assert_eq!(
        contract.commit_outgoing_transaction(&tx, 4, 0),
        Err(AccountError::InvalidForSender)
    );

    // All valid.
    let tx = make_drop_transaction(validator_id.clone(), &bls_pair, 99_999_900);
    assert_eq!(AccountType::verify_outgoing_transaction(&tx), Ok(()));
    assert_eq!(contract.check_outgoing_transaction(&tx, 3000, 0), Ok(()));
    let receipt = contract.commit_outgoing_transaction(&tx, 3000, 0).unwrap();
    assert!(receipt.is_some());

    // Verify contract.
    assert_eq!(contract.balance, Coin::from_u64_unchecked(100_000_000));
    assert_eq!(contract.active_validators_sorted.len(), 0);
    assert_eq!(contract.active_validators_by_id.len(), 0);
    assert_eq!(contract.inactive_validators_by_id.len(), 0);
    assert_eq!(contract.inactive_stake_by_address.len(), 1);
    let inactive_stake = contract
        .inactive_stake_by_address
        .get(&Address::from(&key_pair))
        .unwrap();
    assert_eq!(
        inactive_stake.balance,
        Coin::from_u64_unchecked(100_000_000)
    );
    assert_eq!(inactive_stake.retire_time, 3);

    // Check revert.
    let mut contract_copy = contract.clone();
    assert_eq!(
        contract_copy.revert_outgoing_transaction(&tx, 2, 0, receipt.as_ref()),
        Ok(())
    );

    // Verify contract.
    assert_eq!(contract_copy.balance, Coin::from_u64_unchecked(200_000_000));
    assert_eq!(contract_copy.active_validators_sorted.len(), 0);
    assert_eq!(contract_copy.active_validators_by_id.len(), 0);
    assert_eq!(contract_copy.inactive_validators_by_id.len(), 1);
    assert_eq!(contract_copy.inactive_stake_by_address.len(), 0);
    let validator = contract_copy.get_validator(&validator_id).unwrap();
    assert_eq!(validator.balance, Coin::from_u64_unchecked(200_000_000));
    assert_eq!(validator.active_stake_by_address.read().len(), 1);
    assert_eq!(&validator.validator_key, &validator_key);
    assert_eq!(validator.reward_address, Address::from([3u8; 20]));
}

#[test]
fn it_can_manage_stake() {
    let bls_pair = bls_key_pair();
    let key_pair = ed25519_key_pair();
    let mut contract = make_empty_contract();
    let validator_key = bls_pair.public_key.compress();
    let validator_id: ValidatorId = [0; 20].into();
    let staker_address = Address::from(&key_pair);

    // This test is supposed to test all actions managing stake.
    // To this end, we:
    // 1. Stake.
    // 2. Retire stake.
    // 3. Re-activate stake.
    // 4. Unstake.
    // All actions are applied and reverted for test purposes.
    // Also, we test all actions on active and inactive validators.

    contract
        .create_validator(
            validator_id.clone(),
            validator_key,
            Address::from([3u8; 20]),
            Coin::from_u64_unchecked(100_000_000),
        )
        .unwrap();
    let contract_backup = contract.clone();

    // --- Active validator ---
    // 1. Stake.
    let tx = make_incoming_transaction(
        IncomingStakingTransactionData::Stake {
            validator_id: validator_id.clone(),
            staker_address: None,
        },
        100_000_000,
    );
    assert_eq!(AccountType::verify_incoming_transaction(&tx), Ok(()));
    assert_eq!(
        StakingContract::check_incoming_transaction(&tx, 1, 0),
        Ok(())
    );
    assert_eq!(contract.commit_incoming_transaction(&tx, 1, 0), Ok(None));

    // Verify contract.
    assert_eq!(contract.balance, Coin::from_u64_unchecked(200_000_000));
    assert_eq!(contract.active_validators_sorted.len(), 1);
    assert_eq!(contract.active_validators_by_id.len(), 1);
    assert_eq!(contract.inactive_validators_by_id.len(), 0);
    assert_eq!(contract.inactive_stake_by_address.len(), 0);
    let validator = contract.get_validator(&validator_id).unwrap();
    assert_eq!(validator.balance, Coin::from_u64_unchecked(200_000_000));
    assert_eq!(validator.active_stake_by_address.read().len(), 1);
    assert_eq!(
        validator
            .active_stake_by_address
            .read()
            .get(&staker_address),
        Some(&Coin::from_u64_unchecked(100_000_000))
    );

    // Stake a second time.
    assert_eq!(AccountType::verify_incoming_transaction(&tx), Ok(()));
    assert_eq!(
        StakingContract::check_incoming_transaction(&tx, 1, 0),
        Ok(())
    );
    assert_eq!(contract.commit_incoming_transaction(&tx, 1, 0), Ok(None));

    // Verify contract.
    assert_eq!(contract.balance, Coin::from_u64_unchecked(300_000_000));
    assert_eq!(contract.active_validators_sorted.len(), 1);
    assert_eq!(contract.active_validators_by_id.len(), 1);
    assert_eq!(contract.inactive_validators_by_id.len(), 0);
    assert_eq!(contract.inactive_stake_by_address.len(), 0);
    let validator = contract.get_validator(&validator_id).unwrap();
    assert_eq!(validator.balance, Coin::from_u64_unchecked(300_000_000));
    assert_eq!(validator.active_stake_by_address.read().len(), 1);
    assert_eq!(
        validator
            .active_stake_by_address
            .read()
            .get(&staker_address),
        Some(&Coin::from_u64_unchecked(200_000_000))
    );

    // Test stake for other address.
    let mut contract_copy = contract.clone();
    let tx2 = make_incoming_transaction(
        IncomingStakingTransactionData::Stake {
            validator_id: validator_id.clone(),
            staker_address: Some(Address::from([3u8; 20])),
        },
        100_000_000,
    );
    assert_eq!(AccountType::verify_incoming_transaction(&tx2), Ok(()));
    assert_eq!(
        StakingContract::check_incoming_transaction(&tx2, 1, 0),
        Ok(())
    );
    assert_eq!(
        contract_copy.commit_incoming_transaction(&tx2, 1, 0),
        Ok(None)
    );

    // Verify contract.
    assert_eq!(contract_copy.balance, Coin::from_u64_unchecked(400_000_000));
    assert_eq!(contract_copy.active_validators_sorted.len(), 1);
    assert_eq!(contract_copy.active_validators_by_id.len(), 1);
    assert_eq!(contract_copy.inactive_validators_by_id.len(), 0);
    assert_eq!(contract_copy.inactive_stake_by_address.len(), 0);
    let validator = contract_copy.get_validator(&validator_id).unwrap();
    assert_eq!(validator.balance, Coin::from_u64_unchecked(400_000_000));
    assert_eq!(validator.active_stake_by_address.read().len(), 2);
    assert_eq!(
        validator
            .active_stake_by_address
            .read()
            .get(&staker_address),
        Some(&Coin::from_u64_unchecked(200_000_000))
    );
    assert_eq!(
        validator
            .active_stake_by_address
            .read()
            .get(&Address::from([3u8; 20])),
        Some(&Coin::from_u64_unchecked(100_000_000))
    );

    // Revert one stake.
    assert_eq!(
        contract.revert_incoming_transaction(&tx, 1, 0, None),
        Ok(())
    );

    // Verify contract.
    assert_eq!(contract.balance, Coin::from_u64_unchecked(200_000_000));
    assert_eq!(contract.active_validators_sorted.len(), 1);
    assert_eq!(contract.active_validators_by_id.len(), 1);
    assert_eq!(contract.inactive_validators_by_id.len(), 0);
    assert_eq!(contract.inactive_stake_by_address.len(), 0);
    let validator = contract.get_validator(&validator_id).unwrap();
    assert_eq!(validator.balance, Coin::from_u64_unchecked(200_000_000));
    assert_eq!(validator.active_stake_by_address.read().len(), 1);
    assert_eq!(
        validator
            .active_stake_by_address
            .read()
            .get(&staker_address),
        Some(&Coin::from_u64_unchecked(100_000_000))
    );

    // Revert stakes.
    let mut contract_copy = contract.clone();
    assert_eq!(
        contract_copy.revert_incoming_transaction(&tx, 1, 0, None),
        Ok(())
    );

    // Verify contract.
    assert_eq!(contract_copy.balance, Coin::from_u64_unchecked(100_000_000));
    assert_eq!(contract_copy.active_validators_sorted.len(), 1);
    assert_eq!(contract_copy.active_validators_by_id.len(), 1);
    assert_eq!(contract_copy.inactive_validators_by_id.len(), 0);
    assert_eq!(contract_copy.inactive_stake_by_address.len(), 0);
    let validator = contract_copy.get_validator(&validator_id).unwrap();
    assert_eq!(validator.balance, Coin::from_u64_unchecked(100_000_000));
    assert_eq!(validator.active_stake_by_address.read().len(), 0);

    // 2. Retire stake.
    let tx = make_self_transaction(
        SelfStakingTransactionData::RetireStake(validator_id.clone()),
        49_999_900,
    );
    assert_eq!(AccountType::verify_outgoing_transaction(&tx), Ok(()));
    assert_eq!(contract.check_outgoing_transaction(&tx, 2, 0), Ok(()));
    assert_eq!(contract.commit_outgoing_transaction(&tx, 2, 0), Ok(None));

    assert_eq!(AccountType::verify_incoming_transaction(&tx), Ok(()));
    assert_eq!(
        StakingContract::check_incoming_transaction(&tx, 2, 0),
        Ok(())
    );
    assert_eq!(contract.commit_incoming_transaction(&tx, 2, 0), Ok(None));

    // Verify contract.
    assert_eq!(contract.balance, Coin::from_u64_unchecked(199_999_900));
    assert_eq!(contract.active_validators_sorted.len(), 1);
    assert_eq!(contract.active_validators_by_id.len(), 1);
    assert_eq!(contract.inactive_validators_by_id.len(), 0);
    assert_eq!(contract.inactive_stake_by_address.len(), 1);
    let validator = contract.get_validator(&validator_id).unwrap();
    assert_eq!(validator.balance, Coin::from_u64_unchecked(150_000_000));
    assert_eq!(validator.active_stake_by_address.read().len(), 1);
    assert_eq!(
        validator
            .active_stake_by_address
            .read()
            .get(&staker_address),
        Some(&Coin::from_u64_unchecked(50_000_000))
    );
    let inactive_stake = contract
        .inactive_stake_by_address
        .get(&staker_address)
        .unwrap();
    assert_eq!(inactive_stake.balance, Coin::from_u64_unchecked(49_999_900));
    assert_eq!(inactive_stake.retire_time, 2);

    // Retire rest of stake.
    let mut contract_copy = contract.clone();
    assert_eq!(contract_copy.check_outgoing_transaction(&tx, 3, 0), Ok(()));
    assert_eq!(
        contract_copy.commit_outgoing_transaction(&tx, 3, 0),
        Ok(None)
    );
    let receipt = contract_copy
        .commit_incoming_transaction(&tx, 3, 0)
        .unwrap();
    assert!(receipt.is_some());

    // Verify contract.
    assert_eq!(contract_copy.balance, Coin::from_u64_unchecked(199_999_800));
    assert_eq!(contract_copy.active_validators_sorted.len(), 1);
    assert_eq!(contract_copy.active_validators_by_id.len(), 1);
    assert_eq!(contract_copy.inactive_validators_by_id.len(), 0);
    assert_eq!(contract_copy.inactive_stake_by_address.len(), 1);
    let validator = contract_copy.get_validator(&validator_id).unwrap();
    assert_eq!(validator.balance, Coin::from_u64_unchecked(100_000_000));
    assert_eq!(validator.active_stake_by_address.read().len(), 0);
    let inactive_stake = contract_copy
        .inactive_stake_by_address
        .get(&staker_address)
        .unwrap();
    assert_eq!(inactive_stake.balance, Coin::from_u64_unchecked(99_999_800));
    assert_eq!(inactive_stake.retire_time, 3);

    // Revert retire.
    assert_eq!(
        contract_copy.revert_incoming_transaction(&tx, 3, 0, receipt.as_ref()),
        Ok(())
    );
    assert_eq!(
        contract_copy.revert_outgoing_transaction(&tx, 3, 0, None),
        Ok(())
    );

    // Verify contract.
    assert_eq!(contract_copy.balance, Coin::from_u64_unchecked(199_999_900));
    assert_eq!(contract_copy.active_validators_sorted.len(), 1);
    assert_eq!(contract_copy.active_validators_by_id.len(), 1);
    assert_eq!(contract_copy.inactive_validators_by_id.len(), 0);
    assert_eq!(contract_copy.inactive_stake_by_address.len(), 1);
    let validator = contract_copy.get_validator(&validator_id).unwrap();
    assert_eq!(validator.balance, Coin::from_u64_unchecked(150_000_000));
    assert_eq!(validator.active_stake_by_address.read().len(), 1);
    assert_eq!(
        validator
            .active_stake_by_address
            .read()
            .get(&staker_address),
        Some(&Coin::from_u64_unchecked(50_000_000))
    );
    let inactive_stake = contract_copy
        .inactive_stake_by_address
        .get(&staker_address)
        .unwrap();
    assert_eq!(inactive_stake.balance, Coin::from_u64_unchecked(49_999_900));
    assert_eq!(inactive_stake.retire_time, 2);

    // Revert second retire.
    assert_eq!(
        contract_copy.revert_incoming_transaction(&tx, 2, 0, None),
        Ok(())
    );
    assert_eq!(
        contract_copy.revert_outgoing_transaction(&tx, 2, 0, None),
        Ok(())
    );

    // Verify contract.
    assert_eq!(contract_copy.balance, Coin::from_u64_unchecked(200_000_000));
    assert_eq!(contract_copy.active_validators_sorted.len(), 1);
    assert_eq!(contract_copy.active_validators_by_id.len(), 1);
    assert_eq!(contract_copy.inactive_validators_by_id.len(), 0);
    assert_eq!(contract_copy.inactive_stake_by_address.len(), 0);
    let validator = contract_copy.get_validator(&validator_id).unwrap();
    assert_eq!(validator.balance, Coin::from_u64_unchecked(200_000_000));
    assert_eq!(validator.active_stake_by_address.read().len(), 1);
    assert_eq!(
        validator
            .active_stake_by_address
            .read()
            .get(&staker_address),
        Some(&Coin::from_u64_unchecked(100_000_000))
    );

    // 3. Re-activate stake.
    // Create another validator.
    let bls_pair2 = BlsKeyPair::from(
        BlsSecretKey::deserialize_from_vec(
            &hex::decode(
                "c37da6085717a01d58f86a091579a625675fdc9cecb6e38404c0c21e8b0b570c38\
            b0c414d52bd80a2594497e6d51b9a58c2e8d21976c4482ebbee8dcf7b92f7c1988eb075d3c3f5ecad823d6c\
            37abfd02c20c467493d544a8e1ab1dd367e0100",
            )
            .unwrap(),
        )
        .unwrap(),
    );
    let validator_key2 = bls_pair2.public_key.compress();
    let validator_id2: ValidatorId = [2; 20].into();
    contract
        .create_validator(
            validator_id2.clone(),
            validator_key2,
            Address::from([3u8; 20]),
            Coin::from_u64_unchecked(100_000_000),
        )
        .unwrap();

    // Re-activate stake to new validator.
    let tx = make_self_transaction(
        SelfStakingTransactionData::ReactivateStake(validator_id2.clone()),
        29_999_800,
    );
    assert_eq!(AccountType::verify_outgoing_transaction(&tx), Ok(()));
    assert_eq!(contract.check_outgoing_transaction(&tx, 3, 0), Ok(()));
    assert_eq!(contract.commit_outgoing_transaction(&tx, 3, 0), Ok(None));

    assert_eq!(AccountType::verify_incoming_transaction(&tx), Ok(()));
    assert_eq!(
        StakingContract::check_incoming_transaction(&tx, 3, 0),
        Ok(())
    );
    assert_eq!(contract.commit_incoming_transaction(&tx, 3, 0), Ok(None));

    // Verify contract.
    assert_eq!(contract.balance, Coin::from_u64_unchecked(299_999_800));
    assert_eq!(contract.active_validators_sorted.len(), 2);
    assert_eq!(contract.active_validators_by_id.len(), 2);
    assert_eq!(contract.inactive_validators_by_id.len(), 0);
    assert_eq!(contract.inactive_stake_by_address.len(), 1);
    let validator1 = contract.get_validator(&validator_id).unwrap();
    assert_eq!(validator1.balance, Coin::from_u64_unchecked(150_000_000));
    assert_eq!(validator1.active_stake_by_address.read().len(), 1);
    assert_eq!(
        validator1
            .active_stake_by_address
            .read()
            .get(&staker_address),
        Some(&Coin::from_u64_unchecked(50_000_000))
    );
    let validator2 = contract.get_validator(&validator_id2).unwrap();
    assert_eq!(validator2.balance, Coin::from_u64_unchecked(129_999_800));
    assert_eq!(validator2.active_stake_by_address.read().len(), 1);
    assert_eq!(
        validator2
            .active_stake_by_address
            .read()
            .get(&staker_address),
        Some(&Coin::from_u64_unchecked(29_999_800))
    );
    let inactive_stake = contract
        .inactive_stake_by_address
        .get(&staker_address)
        .unwrap();
    assert_eq!(inactive_stake.balance, Coin::from_u64_unchecked(20_000_000));
    assert_eq!(inactive_stake.retire_time, 2);

    // Re-activate rest of stake.
    let mut contract_copy = contract.clone();
    let tx2 = make_self_transaction(
        SelfStakingTransactionData::ReactivateStake(validator_id2.clone()),
        19_999_900,
    );
    assert_eq!(AccountType::verify_outgoing_transaction(&tx2), Ok(()));
    assert_eq!(contract_copy.check_outgoing_transaction(&tx2, 4, 0), Ok(()));
    let receipt = contract_copy
        .commit_outgoing_transaction(&tx2, 4, 0)
        .unwrap();
    assert!(receipt.is_some());

    assert_eq!(AccountType::verify_incoming_transaction(&tx2), Ok(()));
    assert_eq!(
        StakingContract::check_incoming_transaction(&tx2, 4, 0),
        Ok(())
    );
    assert_eq!(
        contract_copy.commit_incoming_transaction(&tx2, 4, 0),
        Ok(None)
    );

    // Verify contract.
    assert_eq!(contract_copy.balance, Coin::from_u64_unchecked(299_999_700));
    assert_eq!(contract_copy.active_validators_sorted.len(), 2);
    assert_eq!(contract_copy.active_validators_by_id.len(), 2);
    assert_eq!(contract_copy.inactive_validators_by_id.len(), 0);
    assert_eq!(contract_copy.inactive_stake_by_address.len(), 0);
    let validator1 = contract_copy.get_validator(&validator_id).unwrap();
    assert_eq!(validator1.balance, Coin::from_u64_unchecked(150_000_000));
    assert_eq!(validator1.active_stake_by_address.read().len(), 1);
    assert_eq!(
        validator1
            .active_stake_by_address
            .read()
            .get(&staker_address),
        Some(&Coin::from_u64_unchecked(50_000_000))
    );
    let validator2 = contract_copy.get_validator(&validator_id2).unwrap();
    assert_eq!(validator2.balance, Coin::from_u64_unchecked(149_999_700));
    assert_eq!(validator2.active_stake_by_address.read().len(), 1);
    assert_eq!(
        validator2
            .active_stake_by_address
            .read()
            .get(&staker_address),
        Some(&Coin::from_u64_unchecked(49_999_700))
    );

    // Revert re-activation one.
    assert_eq!(
        contract_copy.revert_incoming_transaction(&tx2, 4, 0, None),
        Ok(())
    );
    assert_eq!(
        contract_copy.revert_outgoing_transaction(&tx2, 4, 0, receipt.as_ref()),
        Ok(())
    );

    // Verify contract.
    assert_eq!(contract_copy.balance, Coin::from_u64_unchecked(299_999_800));
    assert_eq!(contract_copy.active_validators_sorted.len(), 2);
    assert_eq!(contract_copy.active_validators_by_id.len(), 2);
    assert_eq!(contract_copy.inactive_validators_by_id.len(), 0);
    assert_eq!(contract_copy.inactive_stake_by_address.len(), 1);
    let validator1 = contract_copy.get_validator(&validator_id).unwrap();
    assert_eq!(validator1.balance, Coin::from_u64_unchecked(150_000_000));
    assert_eq!(validator1.active_stake_by_address.read().len(), 1);
    assert_eq!(
        validator1
            .active_stake_by_address
            .read()
            .get(&staker_address),
        Some(&Coin::from_u64_unchecked(50_000_000))
    );
    let validator2 = contract_copy.get_validator(&validator_id2).unwrap();
    assert_eq!(validator2.balance, Coin::from_u64_unchecked(129_999_800));
    assert_eq!(validator2.active_stake_by_address.read().len(), 1);
    assert_eq!(
        validator2
            .active_stake_by_address
            .read()
            .get(&staker_address),
        Some(&Coin::from_u64_unchecked(29_999_800))
    );
    let inactive_stake = contract_copy
        .inactive_stake_by_address
        .get(&staker_address)
        .unwrap();
    assert_eq!(inactive_stake.balance, Coin::from_u64_unchecked(20_000_000));
    assert_eq!(inactive_stake.retire_time, 2);

    // Revert second re-activation.
    assert_eq!(
        contract_copy.revert_incoming_transaction(&tx, 3, 0, None),
        Ok(())
    );
    assert_eq!(
        contract_copy.revert_outgoing_transaction(&tx, 3, 0, None),
        Ok(())
    );

    // Verify contract.
    assert_eq!(contract_copy.balance, Coin::from_u64_unchecked(299_999_900));
    assert_eq!(contract_copy.active_validators_sorted.len(), 2);
    assert_eq!(contract_copy.active_validators_by_id.len(), 2);
    assert_eq!(contract_copy.inactive_validators_by_id.len(), 0);
    assert_eq!(contract_copy.inactive_stake_by_address.len(), 1);
    let validator1 = contract_copy.get_validator(&validator_id).unwrap();
    assert_eq!(validator1.balance, Coin::from_u64_unchecked(150_000_000));
    assert_eq!(validator1.active_stake_by_address.read().len(), 1);
    assert_eq!(
        validator1
            .active_stake_by_address
            .read()
            .get(&staker_address),
        Some(&Coin::from_u64_unchecked(50_000_000))
    );
    let validator2 = contract_copy.get_validator(&validator_id2).unwrap();
    assert_eq!(validator2.balance, Coin::from_u64_unchecked(100_000_000));
    assert_eq!(validator2.active_stake_by_address.read().len(), 0);
    let inactive_stake = contract_copy
        .inactive_stake_by_address
        .get(&staker_address)
        .unwrap();
    assert_eq!(inactive_stake.balance, Coin::from_u64_unchecked(49_999_900));
    assert_eq!(inactive_stake.retire_time, 2);

    // 4. Unstake.
    // Invalid values.
    // a) zero value
    let tx = make_unstake_transaction(&key_pair, 0);
    assert_eq!(
        AccountType::verify_outgoing_transaction(&tx),
        Err(TransactionError::ZeroValue)
    );

    // b) too high value
    let tx = make_unstake_transaction(&key_pair, 200_000_000);
    assert_eq!(AccountType::verify_outgoing_transaction(&tx), Ok(()));
    assert_eq!(
        contract.check_outgoing_transaction(&tx, 3000, 0),
        Err(AccountError::InsufficientFunds {
            needed: Coin::from_u64_unchecked(200_000_100),
            balance: Coin::from_u64_unchecked(20_000_000),
        })
    );
    assert_eq!(
        contract.commit_outgoing_transaction(&tx, 3000, 0),
        Err(AccountError::InsufficientFunds {
            needed: Coin::from_u64_unchecked(200_000_100),
            balance: Coin::from_u64_unchecked(20_000_000),
        })
    );

    // Invalid timing.
    let tx = make_unstake_transaction(&key_pair, 9_999_700);
    assert_eq!(AccountType::verify_outgoing_transaction(&tx), Ok(()));
    assert_eq!(
        contract.check_outgoing_transaction(&tx, 3, 0),
        Err(AccountError::InvalidForSender)
    );
    assert_eq!(
        contract.commit_outgoing_transaction(&tx, 3, 0),
        Err(AccountError::InvalidForSender)
    );

    // All valid.
    assert_eq!(AccountType::verify_outgoing_transaction(&tx), Ok(()));
    assert_eq!(contract.check_outgoing_transaction(&tx, 3000, 0), Ok(()));
    assert_eq!(contract.commit_outgoing_transaction(&tx, 3000, 0), Ok(None));

    // Verify contract.
    assert_eq!(contract.balance, Coin::from_u64_unchecked(290_000_000));
    assert_eq!(contract.active_validators_sorted.len(), 2);
    assert_eq!(contract.active_validators_by_id.len(), 2);
    assert_eq!(contract.inactive_validators_by_id.len(), 0);
    assert_eq!(contract.inactive_stake_by_address.len(), 1);
    let validator1 = contract.get_validator(&validator_id).unwrap();
    assert_eq!(validator1.balance, Coin::from_u64_unchecked(150_000_000));
    assert_eq!(validator1.active_stake_by_address.read().len(), 1);
    assert_eq!(
        validator1
            .active_stake_by_address
            .read()
            .get(&staker_address),
        Some(&Coin::from_u64_unchecked(50_000_000))
    );
    let validator2 = contract.get_validator(&validator_id2).unwrap();
    assert_eq!(validator2.balance, Coin::from_u64_unchecked(129_999_800));
    assert_eq!(validator2.active_stake_by_address.read().len(), 1);
    assert_eq!(
        validator2
            .active_stake_by_address
            .read()
            .get(&staker_address),
        Some(&Coin::from_u64_unchecked(29_999_800))
    );
    let inactive_stake = contract
        .inactive_stake_by_address
        .get(&staker_address)
        .unwrap();
    assert_eq!(inactive_stake.balance, Coin::from_u64_unchecked(10_000_200));
    assert_eq!(inactive_stake.retire_time, 2);

    // Unstake rest.
    let mut contract_copy = contract.clone();
    let tx2 = make_unstake_transaction(&key_pair, 10_000_100);
    assert_eq!(AccountType::verify_outgoing_transaction(&tx2), Ok(()));
    assert_eq!(
        contract_copy.check_outgoing_transaction(&tx2, 3000, 0),
        Ok(())
    );
    let receipt = contract_copy
        .commit_outgoing_transaction(&tx2, 3000, 0)
        .unwrap();
    assert!(receipt.is_some());

    // Verify contract.
    assert_eq!(contract_copy.balance, Coin::from_u64_unchecked(279_999_800));
    assert_eq!(contract_copy.active_validators_sorted.len(), 2);
    assert_eq!(contract_copy.active_validators_by_id.len(), 2);
    assert_eq!(contract_copy.inactive_validators_by_id.len(), 0);
    assert_eq!(contract_copy.inactive_stake_by_address.len(), 0);
    let validator1 = contract_copy.get_validator(&validator_id).unwrap();
    assert_eq!(validator1.balance, Coin::from_u64_unchecked(150_000_000));
    assert_eq!(validator1.active_stake_by_address.read().len(), 1);
    assert_eq!(
        validator1
            .active_stake_by_address
            .read()
            .get(&staker_address),
        Some(&Coin::from_u64_unchecked(50_000_000))
    );
    let validator2 = contract_copy.get_validator(&validator_id2).unwrap();
    assert_eq!(validator2.balance, Coin::from_u64_unchecked(129_999_800));
    assert_eq!(validator2.active_stake_by_address.read().len(), 1);
    assert_eq!(
        validator2
            .active_stake_by_address
            .read()
            .get(&staker_address),
        Some(&Coin::from_u64_unchecked(29_999_800))
    );

    // Revert unstake.
    assert_eq!(
        contract_copy.revert_outgoing_transaction(&tx2, 3000, 0, receipt.as_ref()),
        Ok(())
    );

    // Verify contract.
    assert_eq!(contract_copy.balance, Coin::from_u64_unchecked(290_000_000));
    assert_eq!(contract_copy.active_validators_sorted.len(), 2);
    assert_eq!(contract_copy.active_validators_by_id.len(), 2);
    assert_eq!(contract_copy.inactive_validators_by_id.len(), 0);
    assert_eq!(contract_copy.inactive_stake_by_address.len(), 1);
    let validator1 = contract_copy.get_validator(&validator_id).unwrap();
    assert_eq!(validator1.balance, Coin::from_u64_unchecked(150_000_000));
    assert_eq!(validator1.active_stake_by_address.read().len(), 1);
    assert_eq!(
        validator1
            .active_stake_by_address
            .read()
            .get(&staker_address),
        Some(&Coin::from_u64_unchecked(50_000_000))
    );
    let validator2 = contract_copy.get_validator(&validator_id2).unwrap();
    assert_eq!(validator2.balance, Coin::from_u64_unchecked(129_999_800));
    assert_eq!(validator2.active_stake_by_address.read().len(), 1);
    assert_eq!(
        validator2
            .active_stake_by_address
            .read()
            .get(&staker_address),
        Some(&Coin::from_u64_unchecked(29_999_800))
    );
    let inactive_stake = contract_copy
        .inactive_stake_by_address
        .get(&staker_address)
        .unwrap();
    assert_eq!(inactive_stake.balance, Coin::from_u64_unchecked(10_000_200));
    assert_eq!(inactive_stake.retire_time, 2);

    // Revert first unstake.
    assert_eq!(
        contract_copy.revert_outgoing_transaction(&tx, 3000, 0, None),
        Ok(())
    );

    // Verify contract.
    assert_eq!(contract_copy.balance, Coin::from_u64_unchecked(299_999_800));
    assert_eq!(contract_copy.active_validators_sorted.len(), 2);
    assert_eq!(contract_copy.active_validators_by_id.len(), 2);
    assert_eq!(contract_copy.inactive_validators_by_id.len(), 0);
    assert_eq!(contract_copy.inactive_stake_by_address.len(), 1);
    let validator1 = contract_copy.get_validator(&validator_id).unwrap();
    assert_eq!(validator1.balance, Coin::from_u64_unchecked(150_000_000));
    assert_eq!(validator1.active_stake_by_address.read().len(), 1);
    assert_eq!(
        validator1
            .active_stake_by_address
            .read()
            .get(&staker_address),
        Some(&Coin::from_u64_unchecked(50_000_000))
    );
    let validator2 = contract_copy.get_validator(&validator_id2).unwrap();
    assert_eq!(validator2.balance, Coin::from_u64_unchecked(129_999_800));
    assert_eq!(validator2.active_stake_by_address.read().len(), 1);
    assert_eq!(
        validator2
            .active_stake_by_address
            .read()
            .get(&staker_address),
        Some(&Coin::from_u64_unchecked(29_999_800))
    );
    let inactive_stake = contract_copy
        .inactive_stake_by_address
        .get(&staker_address)
        .unwrap();
    assert_eq!(inactive_stake.balance, Coin::from_u64_unchecked(20_000_000));
    assert_eq!(inactive_stake.retire_time, 2);

    // --------------------------
    // --- Inactive validator ---
    // --------------------------
    let mut contract = contract_backup;
    let tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::RetireValidator {
            validator_id: validator_id.clone(),
            signature: Default::default(),
        },
        0,
        &bls_pair,
    );
    assert_eq!(AccountType::verify_incoming_transaction(&tx), Ok(()));
    assert_eq!(
        StakingContract::check_incoming_transaction(&tx, 2, 0),
        Ok(())
    );
    assert_eq!(contract.commit_incoming_transaction(&tx, 2, 0), Ok(None));

    // 1. Stake.
    let tx = make_incoming_transaction(
        IncomingStakingTransactionData::Stake {
            validator_id: validator_id.clone(),
            staker_address: None,
        },
        100_000_000,
    );
    assert_eq!(AccountType::verify_incoming_transaction(&tx), Ok(()));
    assert_eq!(
        StakingContract::check_incoming_transaction(&tx, 1, 0),
        Ok(())
    );
    assert_eq!(contract.commit_incoming_transaction(&tx, 1, 0), Ok(None));

    // Verify contract.
    assert_eq!(contract.balance, Coin::from_u64_unchecked(200_000_000));
    assert_eq!(contract.active_validators_sorted.len(), 0);
    assert_eq!(contract.active_validators_by_id.len(), 0);
    assert_eq!(contract.inactive_validators_by_id.len(), 1);
    assert_eq!(contract.inactive_stake_by_address.len(), 0);
    let validator = contract.get_validator(&validator_id).unwrap();
    assert_eq!(validator.balance, Coin::from_u64_unchecked(200_000_000));
    assert_eq!(validator.active_stake_by_address.read().len(), 1);
    assert_eq!(
        validator
            .active_stake_by_address
            .read()
            .get(&staker_address),
        Some(&Coin::from_u64_unchecked(100_000_000))
    );

    // Stake a second time.
    assert_eq!(AccountType::verify_incoming_transaction(&tx), Ok(()));
    assert_eq!(
        StakingContract::check_incoming_transaction(&tx, 1, 0),
        Ok(())
    );
    assert_eq!(contract.commit_incoming_transaction(&tx, 1, 0), Ok(None));

    // Verify contract.
    assert_eq!(contract.balance, Coin::from_u64_unchecked(300_000_000));
    assert_eq!(contract.active_validators_sorted.len(), 0);
    assert_eq!(contract.active_validators_by_id.len(), 0);
    assert_eq!(contract.inactive_validators_by_id.len(), 1);
    assert_eq!(contract.inactive_stake_by_address.len(), 0);
    let validator = contract.get_validator(&validator_id).unwrap();
    assert_eq!(validator.balance, Coin::from_u64_unchecked(300_000_000));
    assert_eq!(validator.active_stake_by_address.read().len(), 1);
    assert_eq!(
        validator
            .active_stake_by_address
            .read()
            .get(&staker_address),
        Some(&Coin::from_u64_unchecked(200_000_000))
    );

    // Test stake for other address.
    let mut contract_copy = contract.clone();
    let tx2 = make_incoming_transaction(
        IncomingStakingTransactionData::Stake {
            validator_id: validator_id.clone(),
            staker_address: Some(Address::from([3u8; 20])),
        },
        100_000_000,
    );
    assert_eq!(AccountType::verify_incoming_transaction(&tx2), Ok(()));
    assert_eq!(
        StakingContract::check_incoming_transaction(&tx2, 1, 0),
        Ok(())
    );
    assert_eq!(
        contract_copy.commit_incoming_transaction(&tx2, 1, 0),
        Ok(None)
    );

    // Verify contract.
    assert_eq!(contract_copy.balance, Coin::from_u64_unchecked(400_000_000));
    assert_eq!(contract_copy.active_validators_sorted.len(), 0);
    assert_eq!(contract_copy.active_validators_by_id.len(), 0);
    assert_eq!(contract_copy.inactive_validators_by_id.len(), 1);
    assert_eq!(contract_copy.inactive_stake_by_address.len(), 0);
    let validator = contract_copy.get_validator(&validator_id).unwrap();
    assert_eq!(validator.balance, Coin::from_u64_unchecked(400_000_000));
    assert_eq!(validator.active_stake_by_address.read().len(), 2);
    assert_eq!(
        validator
            .active_stake_by_address
            .read()
            .get(&staker_address),
        Some(&Coin::from_u64_unchecked(200_000_000))
    );
    assert_eq!(
        validator
            .active_stake_by_address
            .read()
            .get(&Address::from([3u8; 20])),
        Some(&Coin::from_u64_unchecked(100_000_000))
    );

    // Revert one stake.
    assert_eq!(
        contract.revert_incoming_transaction(&tx, 1, 0, None),
        Ok(())
    );

    // Verify contract.
    assert_eq!(contract.balance, Coin::from_u64_unchecked(200_000_000));
    assert_eq!(contract.active_validators_sorted.len(), 0);
    assert_eq!(contract.active_validators_by_id.len(), 0);
    assert_eq!(contract.inactive_validators_by_id.len(), 1);
    assert_eq!(contract.inactive_stake_by_address.len(), 0);
    let validator = contract.get_validator(&validator_id).unwrap();
    assert_eq!(validator.balance, Coin::from_u64_unchecked(200_000_000));
    assert_eq!(validator.active_stake_by_address.read().len(), 1);
    assert_eq!(
        validator
            .active_stake_by_address
            .read()
            .get(&staker_address),
        Some(&Coin::from_u64_unchecked(100_000_000))
    );

    // Revert stakes.
    let mut contract_copy = contract.clone();
    assert_eq!(
        contract_copy.revert_incoming_transaction(&tx, 1, 0, None),
        Ok(())
    );

    // Verify contract.
    assert_eq!(contract_copy.balance, Coin::from_u64_unchecked(100_000_000));
    assert_eq!(contract_copy.active_validators_sorted.len(), 0);
    assert_eq!(contract_copy.active_validators_by_id.len(), 0);
    assert_eq!(contract_copy.inactive_validators_by_id.len(), 1);
    assert_eq!(contract_copy.inactive_stake_by_address.len(), 0);
    let validator = contract_copy.get_validator(&validator_id).unwrap();
    assert_eq!(validator.balance, Coin::from_u64_unchecked(100_000_000));
    assert_eq!(validator.active_stake_by_address.read().len(), 0);

    // 2. Retire stake.
    let tx = make_self_transaction(
        SelfStakingTransactionData::RetireStake(validator_id.clone()),
        49_999_900,
    );
    assert_eq!(AccountType::verify_outgoing_transaction(&tx), Ok(()));
    assert_eq!(contract.check_outgoing_transaction(&tx, 2, 0), Ok(()));
    assert_eq!(contract.commit_outgoing_transaction(&tx, 2, 0), Ok(None));

    assert_eq!(AccountType::verify_incoming_transaction(&tx), Ok(()));
    assert_eq!(
        StakingContract::check_incoming_transaction(&tx, 2, 0),
        Ok(())
    );
    assert_eq!(contract.commit_incoming_transaction(&tx, 2, 0), Ok(None));

    // Verify contract.
    assert_eq!(contract.balance, Coin::from_u64_unchecked(199_999_900));
    assert_eq!(contract.active_validators_sorted.len(), 0);
    assert_eq!(contract.active_validators_by_id.len(), 0);
    assert_eq!(contract.inactive_validators_by_id.len(), 1);
    assert_eq!(contract.inactive_stake_by_address.len(), 1);
    let validator = contract.get_validator(&validator_id).unwrap();
    assert_eq!(validator.balance, Coin::from_u64_unchecked(150_000_000));
    assert_eq!(validator.active_stake_by_address.read().len(), 1);
    assert_eq!(
        validator
            .active_stake_by_address
            .read()
            .get(&staker_address),
        Some(&Coin::from_u64_unchecked(50_000_000))
    );
    let inactive_stake = contract
        .inactive_stake_by_address
        .get(&staker_address)
        .unwrap();
    assert_eq!(inactive_stake.balance, Coin::from_u64_unchecked(49_999_900));
    assert_eq!(inactive_stake.retire_time, 2);

    // Retire rest of stake.
    let mut contract_copy = contract.clone();
    assert_eq!(contract_copy.check_outgoing_transaction(&tx, 3, 0), Ok(()));
    assert_eq!(
        contract_copy.commit_outgoing_transaction(&tx, 3, 0),
        Ok(None)
    );
    let receipt = contract_copy
        .commit_incoming_transaction(&tx, 3, 0)
        .unwrap();
    assert!(receipt.is_some());

    // Verify contract.
    assert_eq!(contract_copy.balance, Coin::from_u64_unchecked(199_999_800));
    assert_eq!(contract_copy.active_validators_sorted.len(), 0);
    assert_eq!(contract_copy.active_validators_by_id.len(), 0);
    assert_eq!(contract_copy.inactive_validators_by_id.len(), 1);
    assert_eq!(contract_copy.inactive_stake_by_address.len(), 1);
    let validator = contract_copy.get_validator(&validator_id).unwrap();
    assert_eq!(validator.balance, Coin::from_u64_unchecked(100_000_000));
    assert_eq!(validator.active_stake_by_address.read().len(), 0);
    let inactive_stake = contract_copy
        .inactive_stake_by_address
        .get(&staker_address)
        .unwrap();
    assert_eq!(inactive_stake.balance, Coin::from_u64_unchecked(99_999_800));
    assert_eq!(inactive_stake.retire_time, 3);

    // Revert retire.
    assert_eq!(
        contract_copy.revert_incoming_transaction(&tx, 3, 0, receipt.as_ref()),
        Ok(())
    );
    assert_eq!(
        contract_copy.revert_outgoing_transaction(&tx, 3, 0, None),
        Ok(())
    );

    // Verify contract.
    assert_eq!(contract_copy.balance, Coin::from_u64_unchecked(199_999_900));
    assert_eq!(contract_copy.active_validators_sorted.len(), 0);
    assert_eq!(contract_copy.active_validators_by_id.len(), 0);
    assert_eq!(contract_copy.inactive_validators_by_id.len(), 1);
    assert_eq!(contract_copy.inactive_stake_by_address.len(), 1);
    let validator = contract_copy.get_validator(&validator_id).unwrap();
    assert_eq!(validator.balance, Coin::from_u64_unchecked(150_000_000));
    assert_eq!(validator.active_stake_by_address.read().len(), 1);
    assert_eq!(
        validator
            .active_stake_by_address
            .read()
            .get(&staker_address),
        Some(&Coin::from_u64_unchecked(50_000_000))
    );
    let inactive_stake = contract_copy
        .inactive_stake_by_address
        .get(&staker_address)
        .unwrap();
    assert_eq!(inactive_stake.balance, Coin::from_u64_unchecked(49_999_900));
    assert_eq!(inactive_stake.retire_time, 2);

    // Revert second retire.
    assert_eq!(
        contract_copy.revert_incoming_transaction(&tx, 2, 0, None),
        Ok(())
    );
    assert_eq!(
        contract_copy.revert_outgoing_transaction(&tx, 2, 0, None),
        Ok(())
    );

    // Verify contract.
    assert_eq!(contract_copy.balance, Coin::from_u64_unchecked(200_000_000));
    assert_eq!(contract_copy.active_validators_sorted.len(), 0);
    assert_eq!(contract_copy.active_validators_by_id.len(), 0);
    assert_eq!(contract_copy.inactive_validators_by_id.len(), 1);
    assert_eq!(contract_copy.inactive_stake_by_address.len(), 0);
    let validator = contract_copy.get_validator(&validator_id).unwrap();
    assert_eq!(validator.balance, Coin::from_u64_unchecked(200_000_000));
    assert_eq!(validator.active_stake_by_address.read().len(), 1);
    assert_eq!(
        validator
            .active_stake_by_address
            .read()
            .get(&staker_address),
        Some(&Coin::from_u64_unchecked(100_000_000))
    );

    // 3. Re-activate stake.
    // Create another validator.
    let bls_pair2 = BlsKeyPair::from(
        BlsSecretKey::deserialize_from_vec(
            &hex::decode(
                "c37da6085717a01d58f86a091579a625675fdc9cecb6e38404c0c21e8b0b570c38\
            b0c414d52bd80a2594497e6d51b9a58c2e8d21976c4482ebbee8dcf7b92f7c1988eb075d3c3f5ecad823d6c\
            37abfd02c20c467493d544a8e1ab1dd367e0100",
            )
            .unwrap(),
        )
        .unwrap(),
    );
    let validator_key2 = bls_pair2.public_key.compress();
    contract
        .create_validator(
            validator_id2.clone(),
            validator_key2,
            Address::from([3u8; 20]),
            Coin::from_u64_unchecked(100_000_000),
        )
        .unwrap();

    // Re-activate stake to new validator.
    let tx = make_self_transaction(
        SelfStakingTransactionData::ReactivateStake(validator_id2.clone()),
        29_999_800,
    );
    assert_eq!(AccountType::verify_outgoing_transaction(&tx), Ok(()));
    assert_eq!(contract.check_outgoing_transaction(&tx, 3, 0), Ok(()));
    assert_eq!(contract.commit_outgoing_transaction(&tx, 3, 0), Ok(None));

    assert_eq!(AccountType::verify_incoming_transaction(&tx), Ok(()));
    assert_eq!(
        StakingContract::check_incoming_transaction(&tx, 3, 0),
        Ok(())
    );
    assert_eq!(contract.commit_incoming_transaction(&tx, 3, 0), Ok(None));

    // Verify contract.
    assert_eq!(contract.balance, Coin::from_u64_unchecked(299_999_800));
    assert_eq!(contract.active_validators_sorted.len(), 1);
    assert_eq!(contract.active_validators_by_id.len(), 1);
    assert_eq!(contract.inactive_validators_by_id.len(), 1);
    assert_eq!(contract.inactive_stake_by_address.len(), 1);
    let validator1 = contract.get_validator(&validator_id).unwrap();
    assert_eq!(validator1.balance, Coin::from_u64_unchecked(150_000_000));
    assert_eq!(validator1.active_stake_by_address.read().len(), 1);
    assert_eq!(
        validator1
            .active_stake_by_address
            .read()
            .get(&staker_address),
        Some(&Coin::from_u64_unchecked(50_000_000))
    );
    let validator2 = contract.get_validator(&validator_id2).unwrap();
    assert_eq!(validator2.balance, Coin::from_u64_unchecked(129_999_800));
    assert_eq!(validator2.active_stake_by_address.read().len(), 1);
    assert_eq!(
        validator2
            .active_stake_by_address
            .read()
            .get(&staker_address),
        Some(&Coin::from_u64_unchecked(29_999_800))
    );
    let inactive_stake = contract
        .inactive_stake_by_address
        .get(&staker_address)
        .unwrap();
    assert_eq!(inactive_stake.balance, Coin::from_u64_unchecked(20_000_000));
    assert_eq!(inactive_stake.retire_time, 2);

    // Re-activate rest of stake.
    let mut contract_copy = contract.clone();
    let tx2 = make_self_transaction(
        SelfStakingTransactionData::ReactivateStake(validator_id2.clone()),
        19_999_900,
    );
    assert_eq!(AccountType::verify_outgoing_transaction(&tx2), Ok(()));
    assert_eq!(contract_copy.check_outgoing_transaction(&tx2, 4, 0), Ok(()));
    let receipt = contract_copy
        .commit_outgoing_transaction(&tx2, 4, 0)
        .unwrap();
    assert!(receipt.is_some());

    assert_eq!(AccountType::verify_incoming_transaction(&tx2), Ok(()));
    assert_eq!(
        StakingContract::check_incoming_transaction(&tx2, 4, 0),
        Ok(())
    );
    assert_eq!(
        contract_copy.commit_incoming_transaction(&tx2, 4, 0),
        Ok(None)
    );

    // Verify contract.
    assert_eq!(contract_copy.balance, Coin::from_u64_unchecked(299_999_700));
    assert_eq!(contract_copy.active_validators_sorted.len(), 1);
    assert_eq!(contract_copy.active_validators_by_id.len(), 1);
    assert_eq!(contract_copy.inactive_validators_by_id.len(), 1);
    assert_eq!(contract_copy.inactive_stake_by_address.len(), 0);
    let validator1 = contract_copy.get_validator(&validator_id).unwrap();
    assert_eq!(validator1.balance, Coin::from_u64_unchecked(150_000_000));
    assert_eq!(validator1.active_stake_by_address.read().len(), 1);
    assert_eq!(
        validator1
            .active_stake_by_address
            .read()
            .get(&staker_address),
        Some(&Coin::from_u64_unchecked(50_000_000))
    );
    let validator2 = contract_copy.get_validator(&validator_id2).unwrap();
    assert_eq!(validator2.balance, Coin::from_u64_unchecked(149_999_700));
    assert_eq!(validator2.active_stake_by_address.read().len(), 1);
    assert_eq!(
        validator2
            .active_stake_by_address
            .read()
            .get(&staker_address),
        Some(&Coin::from_u64_unchecked(49_999_700))
    );

    // Revert re-activation one.
    assert_eq!(
        contract_copy.revert_incoming_transaction(&tx2, 4, 0, None),
        Ok(())
    );
    assert_eq!(
        contract_copy.revert_outgoing_transaction(&tx2, 4, 0, receipt.as_ref()),
        Ok(())
    );

    // Verify contract.
    assert_eq!(contract_copy.balance, Coin::from_u64_unchecked(299_999_800));
    assert_eq!(contract_copy.active_validators_sorted.len(), 1);
    assert_eq!(contract_copy.active_validators_by_id.len(), 1);
    assert_eq!(contract_copy.inactive_validators_by_id.len(), 1);
    assert_eq!(contract_copy.inactive_stake_by_address.len(), 1);
    let validator1 = contract_copy.get_validator(&validator_id).unwrap();
    assert_eq!(validator1.balance, Coin::from_u64_unchecked(150_000_000));
    assert_eq!(validator1.active_stake_by_address.read().len(), 1);
    assert_eq!(
        validator1
            .active_stake_by_address
            .read()
            .get(&staker_address),
        Some(&Coin::from_u64_unchecked(50_000_000))
    );
    let validator2 = contract_copy.get_validator(&validator_id2).unwrap();
    assert_eq!(validator2.balance, Coin::from_u64_unchecked(129_999_800));
    assert_eq!(validator2.active_stake_by_address.read().len(), 1);
    assert_eq!(
        validator2
            .active_stake_by_address
            .read()
            .get(&staker_address),
        Some(&Coin::from_u64_unchecked(29_999_800))
    );
    let inactive_stake = contract_copy
        .inactive_stake_by_address
        .get(&staker_address)
        .unwrap();
    assert_eq!(inactive_stake.balance, Coin::from_u64_unchecked(20_000_000));
    assert_eq!(inactive_stake.retire_time, 2);

    // Revert second re-activation.
    assert_eq!(
        contract_copy.revert_incoming_transaction(&tx, 3, 0, None),
        Ok(())
    );
    assert_eq!(
        contract_copy.revert_outgoing_transaction(&tx, 3, 0, None),
        Ok(())
    );

    // Verify contract.
    assert_eq!(contract_copy.balance, Coin::from_u64_unchecked(299_999_900));
    assert_eq!(contract_copy.active_validators_sorted.len(), 1);
    assert_eq!(contract_copy.active_validators_by_id.len(), 1);
    assert_eq!(contract_copy.inactive_validators_by_id.len(), 1);
    assert_eq!(contract_copy.inactive_stake_by_address.len(), 1);
    let validator1 = contract_copy.get_validator(&validator_id).unwrap();
    assert_eq!(validator1.balance, Coin::from_u64_unchecked(150_000_000));
    assert_eq!(validator1.active_stake_by_address.read().len(), 1);
    assert_eq!(
        validator1
            .active_stake_by_address
            .read()
            .get(&staker_address),
        Some(&Coin::from_u64_unchecked(50_000_000))
    );
    let validator2 = contract_copy.get_validator(&validator_id2).unwrap();
    assert_eq!(validator2.balance, Coin::from_u64_unchecked(100_000_000));
    assert_eq!(validator2.active_stake_by_address.read().len(), 0);
    let inactive_stake = contract_copy
        .inactive_stake_by_address
        .get(&staker_address)
        .unwrap();
    assert_eq!(inactive_stake.balance, Coin::from_u64_unchecked(49_999_900));
    assert_eq!(inactive_stake.retire_time, 2);

    // 4. Unstake.
    // Invalid values.
    // a) zero value
    let tx = make_unstake_transaction(&key_pair, 0);
    assert_eq!(
        AccountType::verify_outgoing_transaction(&tx),
        Err(TransactionError::ZeroValue)
    );

    // b) too high value
    let tx = make_unstake_transaction(&key_pair, 200_000_000);
    assert_eq!(AccountType::verify_outgoing_transaction(&tx), Ok(()));
    assert_eq!(
        contract.check_outgoing_transaction(&tx, 3000, 0),
        Err(AccountError::InsufficientFunds {
            needed: Coin::from_u64_unchecked(200_000_100),
            balance: Coin::from_u64_unchecked(20_000_000),
        })
    );
    assert_eq!(
        contract.commit_outgoing_transaction(&tx, 3000, 0),
        Err(AccountError::InsufficientFunds {
            needed: Coin::from_u64_unchecked(200_000_100),
            balance: Coin::from_u64_unchecked(20_000_000),
        })
    );

    // Invalid timing.
    let tx = make_unstake_transaction(&key_pair, 9_999_700);
    assert_eq!(AccountType::verify_outgoing_transaction(&tx), Ok(()));
    assert_eq!(
        contract.check_outgoing_transaction(&tx, 3, 0),
        Err(AccountError::InvalidForSender)
    );
    assert_eq!(
        contract.commit_outgoing_transaction(&tx, 3, 0),
        Err(AccountError::InvalidForSender)
    );

    // All valid.
    assert_eq!(AccountType::verify_outgoing_transaction(&tx), Ok(()));
    assert_eq!(contract.check_outgoing_transaction(&tx, 3000, 0), Ok(()));
    assert_eq!(contract.commit_outgoing_transaction(&tx, 3000, 0), Ok(None));

    // Verify contract.
    assert_eq!(contract.balance, Coin::from_u64_unchecked(290_000_000));
    assert_eq!(contract.active_validators_sorted.len(), 1);
    assert_eq!(contract.active_validators_by_id.len(), 1);
    assert_eq!(contract.inactive_validators_by_id.len(), 1);
    assert_eq!(contract.inactive_stake_by_address.len(), 1);
    let validator1 = contract.get_validator(&validator_id).unwrap();
    assert_eq!(validator1.balance, Coin::from_u64_unchecked(150_000_000));
    assert_eq!(validator1.active_stake_by_address.read().len(), 1);
    assert_eq!(
        validator1
            .active_stake_by_address
            .read()
            .get(&staker_address),
        Some(&Coin::from_u64_unchecked(50_000_000))
    );
    let validator2 = contract.get_validator(&validator_id2).unwrap();
    assert_eq!(validator2.balance, Coin::from_u64_unchecked(129_999_800));
    assert_eq!(validator2.active_stake_by_address.read().len(), 1);
    assert_eq!(
        validator2
            .active_stake_by_address
            .read()
            .get(&staker_address),
        Some(&Coin::from_u64_unchecked(29_999_800))
    );
    let inactive_stake = contract
        .inactive_stake_by_address
        .get(&staker_address)
        .unwrap();
    assert_eq!(inactive_stake.balance, Coin::from_u64_unchecked(10_000_200));
    assert_eq!(inactive_stake.retire_time, 2);

    // Unstake rest.
    let mut contract_copy = contract.clone();
    let tx2 = make_unstake_transaction(&key_pair, 10_000_100);
    assert_eq!(AccountType::verify_outgoing_transaction(&tx2), Ok(()));
    assert_eq!(
        contract_copy.check_outgoing_transaction(&tx2, 3000, 0),
        Ok(())
    );
    let receipt = contract_copy
        .commit_outgoing_transaction(&tx2, 3000, 0)
        .unwrap();
    assert!(receipt.is_some());

    // Verify contract.
    assert_eq!(contract_copy.balance, Coin::from_u64_unchecked(279_999_800));
    assert_eq!(contract_copy.active_validators_sorted.len(), 1);
    assert_eq!(contract_copy.active_validators_by_id.len(), 1);
    assert_eq!(contract_copy.inactive_validators_by_id.len(), 1);
    assert_eq!(contract_copy.inactive_stake_by_address.len(), 0);
    let validator1 = contract_copy.get_validator(&validator_id).unwrap();
    assert_eq!(validator1.balance, Coin::from_u64_unchecked(150_000_000));
    assert_eq!(validator1.active_stake_by_address.read().len(), 1);
    assert_eq!(
        validator1
            .active_stake_by_address
            .read()
            .get(&staker_address),
        Some(&Coin::from_u64_unchecked(50_000_000))
    );
    let validator2 = contract_copy.get_validator(&validator_id2).unwrap();
    assert_eq!(validator2.balance, Coin::from_u64_unchecked(129_999_800));
    assert_eq!(validator2.active_stake_by_address.read().len(), 1);
    assert_eq!(
        validator2
            .active_stake_by_address
            .read()
            .get(&staker_address),
        Some(&Coin::from_u64_unchecked(29_999_800))
    );

    // Revert unstake.
    assert_eq!(
        contract_copy.revert_outgoing_transaction(&tx2, 3000, 0, receipt.as_ref()),
        Ok(())
    );

    // Verify contract.
    assert_eq!(contract_copy.balance, Coin::from_u64_unchecked(290_000_000));
    assert_eq!(contract_copy.active_validators_sorted.len(), 1);
    assert_eq!(contract_copy.active_validators_by_id.len(), 1);
    assert_eq!(contract_copy.inactive_validators_by_id.len(), 1);
    assert_eq!(contract_copy.inactive_stake_by_address.len(), 1);
    let validator1 = contract_copy.get_validator(&validator_id).unwrap();
    assert_eq!(validator1.balance, Coin::from_u64_unchecked(150_000_000));
    assert_eq!(validator1.active_stake_by_address.read().len(), 1);
    assert_eq!(
        validator1
            .active_stake_by_address
            .read()
            .get(&staker_address),
        Some(&Coin::from_u64_unchecked(50_000_000))
    );
    let validator2 = contract_copy.get_validator(&validator_id2).unwrap();
    assert_eq!(validator2.balance, Coin::from_u64_unchecked(129_999_800));
    assert_eq!(validator2.active_stake_by_address.read().len(), 1);
    assert_eq!(
        validator2
            .active_stake_by_address
            .read()
            .get(&staker_address),
        Some(&Coin::from_u64_unchecked(29_999_800))
    );
    let inactive_stake = contract_copy
        .inactive_stake_by_address
        .get(&staker_address)
        .unwrap();
    assert_eq!(inactive_stake.balance, Coin::from_u64_unchecked(10_000_200));
    assert_eq!(inactive_stake.retire_time, 2);

    // Revert first unstake.
    assert_eq!(
        contract_copy.revert_outgoing_transaction(&tx, 3000, 0, None),
        Ok(())
    );

    // Verify contract.
    assert_eq!(contract_copy.balance, Coin::from_u64_unchecked(299_999_800));
    assert_eq!(contract_copy.active_validators_sorted.len(), 1);
    assert_eq!(contract_copy.active_validators_by_id.len(), 1);
    assert_eq!(contract_copy.inactive_validators_by_id.len(), 1);
    assert_eq!(contract_copy.inactive_stake_by_address.len(), 1);
    let validator1 = contract_copy.get_validator(&validator_id).unwrap();
    assert_eq!(validator1.balance, Coin::from_u64_unchecked(150_000_000));
    assert_eq!(validator1.active_stake_by_address.read().len(), 1);
    assert_eq!(
        validator1
            .active_stake_by_address
            .read()
            .get(&staker_address),
        Some(&Coin::from_u64_unchecked(50_000_000))
    );
    let validator2 = contract_copy.get_validator(&validator_id2).unwrap();
    assert_eq!(validator2.balance, Coin::from_u64_unchecked(129_999_800));
    assert_eq!(validator2.active_stake_by_address.read().len(), 1);
    assert_eq!(
        validator2
            .active_stake_by_address
            .read()
            .get(&staker_address),
        Some(&Coin::from_u64_unchecked(29_999_800))
    );
    let inactive_stake = contract_copy
        .inactive_stake_by_address
        .get(&staker_address)
        .unwrap();
    assert_eq!(inactive_stake.balance, Coin::from_u64_unchecked(20_000_000));
    assert_eq!(inactive_stake.retire_time, 2);
}

fn make_empty_contract() -> StakingContract {
    StakingContract::default()
}

fn make_sample_contract(
    validator_id: &ValidatorId,
    key_pair: &KeyPair,
    bls_pair: &BlsKeyPair,
) -> StakingContract {
    let mut contract = make_empty_contract();
    contract
        .create_validator(
            validator_id.clone(),
            bls_pair.public_key.compress(),
            Address::from(key_pair),
            Coin::from_u64_unchecked(150_000_000),
        )
        .unwrap();
    contract
        .stake(
            Address::from(key_pair),
            Coin::from_u64_unchecked(150_000_000),
            validator_id,
        )
        .unwrap();

    contract
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
) -> Transaction {
    let mut tx = make_incoming_transaction(data, value);
    tx.data = IncomingStakingTransactionData::set_validator_signature_on_data(
        &tx.data,
        bls_pair.sign(&tx.serialize_content()).compress(),
    )
    .unwrap();

    let private_key =
        PrivateKey::deserialize_from_vec(&hex::decode(STAKER_PRIVATE_KEY).unwrap()).unwrap();
    let key_pair = KeyPair::from(private_key);
    tx.proof = SignatureProof::from(key_pair.public, key_pair.sign(&tx.serialize_content()))
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
        key_pair.public,
        key_pair.sign(&tx.serialize_content()),
    ));
    tx.proof = proof.serialize_to_vec();
    tx
}

fn make_drop_transaction(
    validator_id: ValidatorId,
    key_pair: &BlsKeyPair,
    value: u64,
) -> Transaction {
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
        validator_id,
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
    tx.proof = SignatureProof::from(key_pair.public, key_pair.sign(&tx.serialize_content()))
        .serialize_to_vec();
    tx
}

fn bls_key_pair() -> BlsKeyPair {
    BlsKeyPair::from(
        BlsSecretKey::deserialize_from_vec(&hex::decode(VALIDATOR_SECRET_KEY).unwrap()).unwrap(),
    )
}

fn ed25519_key_pair() -> KeyPair {
    KeyPair::from(
        PrivateKey::deserialize_from_vec(&hex::decode(STAKER_PRIVATE_KEY).unwrap()).unwrap(),
    )
}
