use std::{
    collections::{BTreeMap, BTreeSet},
    convert::TryInto,
};

use nimiq_account::*;
use nimiq_bls::{
    CompressedPublicKey as BlsPublicKey, KeyPair as BlsKeyPair, SecretKey as BlsSecretKey,
};
use nimiq_collections::BitSet;
use nimiq_database::{traits::Database, volatile::VolatileDatabase};
use nimiq_hash::Blake2bHash;
use nimiq_keys::{Address, KeyPair, PrivateKey, PublicKey};
use nimiq_primitives::{
    account::{AccountError, AccountType},
    coin::Coin,
    networks::NetworkId,
    policy::Policy,
    slots::SlashedSlot,
};
use nimiq_serde::{Deserialize, Serialize};
use nimiq_test_log::test;
use nimiq_test_utils::test_rng::test_rng;
use nimiq_transaction::{
    account::staking_contract::{IncomingStakingTransactionData, OutgoingStakingTransactionProof},
    inherent::Inherent,
    SignatureProof, Transaction,
};
use nimiq_utils::key_rng::SecureGenerate;

const VALIDATOR_ADDRESS: &str = "83fa05dbe31f85e719f4c4fd67ebdba2e444d9f8";
const VALIDATOR_PRIVATE_KEY: &str =
    "d0fbb3690f5308f457e245a3cc65ae8d6945155eadcac60d489ffc5583a60b9b";

const VALIDATOR_SIGNING_KEY: &str =
    "b300481ddd7af6be3cf5c123b7af2c21f87f4ac808c8b0e622eb85826124a844";
const VALIDATOR_SIGNING_SECRET_KEY: &str =
    "84c961b11b52a8244ffc5e9d0965bc2dfa6764970f8e4989d45901de401baf27";

const VALIDATOR_VOTING_KEY: &str = "713c60858b5c72adcf8b72b4dbea959d042769dcc93a0190e4b8aec92283548138833950aa214d920c17d3d19de27f6176d9fb21620edae76ad398670e17d5eba2f494b9b6901d457592ea68f9d35380c857ba44856ae037aff272ad6c1900442b426dde0bc53431e9ce5807f7ec4a05e71ce4a1e7e7b2511891521c4d3fd975764e3031ef646d48fa881ad88240813d40e533788f0dac2bc4d4c25db7b108c67dd28b7ec4c240cdc044badcaed7860a5d3da42ef860ed25a6db9c07be000a7f504f6d1b24ac81642206d5996b20749a156d7b39f851e60f228b19eef3fb3547469f03fc9764f5f68bc88e187ffee0f43f169acde847c78ea88029cdb19b91dd9562d60b607dd0347d67a0e33286c8908e4e9579a42685da95f06a9201";
const VALIDATOR_VOTING_SECRET_KEY: &str =
        "65100f4aa301ded3d9868c3d76052dd0dfede426b51af371dcd8a4a076f11651c86286d2891063ce7b78217a6e163f38ebfde7eb9dcbf5927b2278b00d77329141d44f070620dd6b995455a6cdfe8eee03f657ff255cfb8fb3460ce1135701";

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

    let mut current_batch_lost_rewards = BitSet::new();
    current_batch_lost_rewards.insert(0);
    current_batch_lost_rewards.insert(10);

    let mut previous_batch_lost_rewards = BitSet::new();
    previous_batch_lost_rewards.insert(100);
    previous_batch_lost_rewards.insert(101);
    previous_batch_lost_rewards.insert(102);
    previous_batch_lost_rewards.insert(104);

    let mut b_set = BTreeSet::new();
    b_set.insert(0);
    b_set.insert(10);
    let mut current_epoch_disabled_slots = BTreeMap::new();
    current_epoch_disabled_slots.insert(Address::from([1u8; 20]), b_set);

    let mut b_set = BTreeSet::new();
    b_set.insert(100);
    b_set.insert(101);
    b_set.insert(102);
    b_set.insert(104);
    let mut previous_epoch_disabled_slots = BTreeMap::new();
    previous_epoch_disabled_slots.insert(Address::from([2u8; 20]), b_set);

    let contract = StakingContract {
        balance: Coin::from_u64_unchecked(300_000_000),
        active_validators,
        current_batch_lost_rewards,
        previous_batch_lost_rewards,
        current_epoch_disabled_slots,
        previous_epoch_disabled_slots,
    };

    assert_eq!(&hex::encode(contract.serialize_to_vec()), "");
}

#[test]
fn can_iter_stakers() {
    let env = VolatileDatabase::new(10).unwrap();
    let accounts = Accounts::new(env.clone());
    let data_store = accounts.data_store(&Policy::STAKING_CONTRACT_ADDRESS);
    let mut db_txn = env.write_transaction();

    let staking_contract = make_sample_contract(data_store.write(&mut db_txn), true);

    let stakers =
        staking_contract.get_stakers_for_validator(&data_store.read(&db_txn), &validator_address());

    assert_eq!(stakers.len(), 1);
    assert_eq!(stakers[0].balance, Coin::from_u64_unchecked(150_000_000));
}

#[test]
fn it_can_de_serialize_a_staking_contract() {
    let contract_1 = StakingContract::default();
    let contract_1a: StakingContract =
        Deserialize::deserialize_from_vec(&contract_1.serialize_to_vec()).unwrap();

    assert_eq!(contract_1, contract_1a);

    let balance = Coin::from_u64_unchecked(300_000_000);
    let mut current_batch_lost_rewards = BitSet::new();
    current_batch_lost_rewards.insert(1);
    current_batch_lost_rewards.insert(2);
    let mut active_validators: BTreeMap<Address, Coin> = BTreeMap::new();
    active_validators.insert(Address::START_ADDRESS, Coin::MAX);
    let mut previous_batch_lost_rewards = current_batch_lost_rewards.clone();
    previous_batch_lost_rewards.insert(3);
    previous_batch_lost_rewards.insert(4);
    let mut current_epoch_disabled_slots = BTreeMap::new();
    current_epoch_disabled_slots.insert(Address::START_ADDRESS, BTreeSet::new());
    let previous_epoch_disabled_slots = current_epoch_disabled_slots.clone();

    let contract_2 = StakingContract {
        balance,
        active_validators,
        current_batch_lost_rewards,
        previous_batch_lost_rewards,
        current_epoch_disabled_slots,
        previous_epoch_disabled_slots,
    };
    let contract_2a: StakingContract =
        Deserialize::deserialize_from_vec(&contract_2.serialize_to_vec()).unwrap();

    assert_eq!(contract_2, contract_2a);
}

#[test]
fn can_get_it() {
    let env = VolatileDatabase::new(10).unwrap();
    let accounts = Accounts::new(env.clone());
    let data_store = accounts.data_store(&Policy::STAKING_CONTRACT_ADDRESS);
    let mut db_txn = env.write_transaction();

    let staking_contract = make_sample_contract(data_store.write(&mut db_txn), true);

    assert_eq!(
        staking_contract.balance,
        Coin::from_u64_unchecked(150_000_000 + Policy::VALIDATOR_DEPOSIT)
    );

    let validator = staking_contract
        .get_validator(&data_store.read(&db_txn), &validator_address())
        .expect("Validator should exist");

    assert_eq!(
        validator.total_stake,
        Coin::from_u64_unchecked(150_000_000 + Policy::VALIDATOR_DEPOSIT)
    );

    let staker = staking_contract
        .get_staker(&data_store.read(&db_txn), &staker_address())
        .expect("Staker should exist");

    assert_eq!(staker.balance, Coin::from_u64_unchecked(150_000_000));
}

#[test]
fn create_validator_works() {
    let env = VolatileDatabase::new(10).unwrap();
    let accounts = Accounts::new(env.clone());
    let data_store = accounts.data_store(&Policy::STAKING_CONTRACT_ADDRESS);
    let block_state = BlockState::new(1, 1);
    let mut db_txn = env.write_transaction();

    let mut staking_contract = make_empty_contract();

    let validator_address = validator_address();
    let cold_keypair = ed25519_key_pair(VALIDATOR_PRIVATE_KEY);
    let signing_key = ed25519_public_key(VALIDATOR_SIGNING_KEY);
    let voting_key = bls_public_key(VALIDATOR_VOTING_KEY);
    let voting_keypair = bls_key_pair(VALIDATOR_VOTING_SECRET_KEY);
    let reward_address = Address::from([3u8; 20]);

    assert_eq!(voting_key.uncompress().unwrap(), voting_keypair.public_key);

    // Works in the valid case.
    let tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::CreateValidator {
            signing_key,
            voting_key: voting_key.clone(),
            proof_of_knowledge: voting_keypair
                .sign(&voting_key.serialize_to_vec())
                .compress(),
            reward_address: reward_address.clone(),
            signal_data: None,
            proof: SignatureProof::default(),
        },
        Policy::VALIDATOR_DEPOSIT,
        &cold_keypair,
    );

    let mut tx_logger = TransactionLog::empty();
    let receipt = staking_contract
        .commit_incoming_transaction(
            &tx,
            &block_state,
            data_store.write(&mut db_txn),
            &mut tx_logger,
        )
        .expect("Failed to commit transaction");

    assert_eq!(receipt, None);
    assert_eq!(
        tx_logger.logs,
        vec![Log::CreateValidator {
            validator_address: validator_address.clone(),
            reward_address,
        }]
    );

    let validator = staking_contract
        .get_validator(&data_store.read(&db_txn), &validator_address)
        .expect("Validator should exist");

    assert_eq!(validator.address, validator_address);
    assert_eq!(validator.signing_key, signing_key);
    assert_eq!(validator.voting_key, voting_key);
    assert_eq!(validator.reward_address, Address::from([3u8; 20]));
    assert_eq!(validator.signal_data, None);
    assert_eq!(
        validator.total_stake,
        Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT)
    );
    assert_eq!(validator.num_stakers, 0);
    assert_eq!(validator.inactive_since, None);

    assert_eq!(
        staking_contract.balance,
        Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT)
    );

    assert_eq!(
        staking_contract.active_validators.get(&validator_address),
        Some(&Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT))
    );

    // Doesn't work when the validator already exists.
    let block_state = BlockState::new(2, 2);
    assert_eq!(
        staking_contract.commit_incoming_transaction(
            &tx,
            &block_state,
            data_store.write(&mut db_txn),
            &mut TransactionLog::empty()
        ),
        Err(AccountError::AlreadyExistentAddress {
            address: validator_address.clone()
        })
    );

    // Revert the transaction.
    let mut tx_logger = TransactionLog::empty();
    staking_contract
        .revert_incoming_transaction(
            &tx,
            &block_state,
            None,
            data_store.write(&mut db_txn),
            &mut tx_logger,
        )
        .expect("Failed to revert transaction");
    assert_eq!(
        tx_logger.logs,
        vec![Log::CreateValidator {
            validator_address: validator_address.clone(),
            reward_address: validator.reward_address,
        }]
    );

    assert_eq!(
        staking_contract.get_validator(&data_store.read(&db_txn), &validator_address),
        None
    );

    assert_eq!(staking_contract.balance, Coin::ZERO);

    assert_eq!(
        staking_contract.active_validators.get(&validator_address),
        None
    );
}

#[test]
fn update_validator_works() {
    let mut rng = test_rng(false);
    let env = VolatileDatabase::new(10).unwrap();
    let accounts = Accounts::new(env.clone());
    let data_store = accounts.data_store(&Policy::STAKING_CONTRACT_ADDRESS);
    let block_state = BlockState::new(2, 2);
    let mut db_txn = env.write_transaction();

    let mut staking_contract = make_sample_contract(data_store.write(&mut db_txn), true);

    let validator_address = validator_address();
    let cold_keypair = ed25519_key_pair(VALIDATOR_PRIVATE_KEY);
    let new_voting_keypair = BlsKeyPair::generate(&mut rng);
    let new_reward_address = Some(Address::from([77u8; 20]));

    // Works in the valid case.
    let tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::UpdateValidator {
            new_signing_key: Some(PublicKey::from([88u8; 32])),
            new_voting_key: Some(new_voting_keypair.public_key.compress()),
            new_reward_address: new_reward_address.clone(),
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

    let mut tx_logger = TransactionLog::empty();
    let receipt = staking_contract
        .commit_incoming_transaction(
            &tx,
            &block_state,
            data_store.write(&mut db_txn),
            &mut tx_logger,
        )
        .expect("Failed to commit transaction");

    let old_signing_key = ed25519_public_key(VALIDATOR_SIGNING_KEY);
    let old_voting_key = bls_public_key(VALIDATOR_VOTING_KEY);
    let old_reward_address = validator_address.clone();

    let expected_receipt = UpdateValidatorReceipt {
        old_signing_key,
        old_voting_key: old_voting_key.clone(),
        old_reward_address: old_reward_address.clone(),
        old_signal_data: None,
    };
    assert_eq!(receipt, Some(expected_receipt.into()));

    assert_eq!(
        tx_logger.logs,
        vec![Log::UpdateValidator {
            validator_address: validator_address.clone(),
            old_reward_address: old_reward_address.clone(),
            new_reward_address: new_reward_address.clone(),
        }]
    );

    let validator = staking_contract
        .get_validator(&data_store.read(&db_txn), &validator_address)
        .expect("Validator should exist");

    assert_eq!(validator.address, validator_address);
    assert_eq!(validator.signing_key, PublicKey::from([88u8; 32]));
    assert_eq!(
        validator.voting_key,
        new_voting_keypair.public_key.compress()
    );
    assert_eq!(validator.reward_address, Address::from([77u8; 20]));
    assert_eq!(validator.signal_data, Some(Blake2bHash::default()));
    assert_eq!(
        validator.total_stake,
        Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT + 150_000_000)
    );
    assert_eq!(validator.num_stakers, 1);
    assert_eq!(validator.inactive_since, None);

    // Revert the transaction.
    let mut tx_logger = TransactionLog::empty();
    staking_contract
        .revert_incoming_transaction(
            &tx,
            &block_state,
            receipt,
            data_store.write(&mut db_txn),
            &mut tx_logger,
        )
        .expect("Failed to revert transaction");

    assert_eq!(
        tx_logger.logs,
        vec![Log::UpdateValidator {
            validator_address: validator_address.clone(),
            old_reward_address: old_reward_address.clone(),
            new_reward_address,
        }]
    );

    let validator = staking_contract
        .get_validator(&data_store.read(&db_txn), &validator_address)
        .expect("Validator should exist");

    assert_eq!(validator.address, validator_address);
    assert_eq!(validator.signing_key, old_signing_key);
    assert_eq!(validator.voting_key, old_voting_key);
    assert_eq!(validator.reward_address, old_reward_address);
    assert_eq!(validator.signal_data, None);
    assert_eq!(
        validator.total_stake,
        Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT + 150_000_000)
    );
    assert_eq!(validator.num_stakers, 1);
    assert_eq!(validator.inactive_since, None);

    // Try with a non-existent validator.
    let fake_keypair = ed25519_key_pair(STAKER_PRIVATE_KEY);

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
        &fake_keypair,
    );

    assert_eq!(
        staking_contract.commit_incoming_transaction(
            &tx,
            &block_state,
            data_store.write(&mut db_txn),
            &mut TransactionLog::empty(),
        ),
        Err(AccountError::NonExistentAddress {
            address: staker_address()
        })
    );
}

#[test]
fn deactivate_validator_works() {
    let env = VolatileDatabase::new(10).unwrap();
    let accounts = Accounts::new(env.clone());
    let data_store = accounts.data_store(&Policy::STAKING_CONTRACT_ADDRESS);
    let block_state = BlockState::new(2, 2);
    let mut db_txn = env.write_transaction();

    let mut staking_contract = make_sample_contract(data_store.write(&mut db_txn), true);

    let validator_address = validator_address();
    let cold_keypair = ed25519_key_pair(VALIDATOR_PRIVATE_KEY);
    let signing_key = ed25519_public_key(VALIDATOR_SIGNING_KEY);
    let signing_keypair = ed25519_key_pair(VALIDATOR_SIGNING_SECRET_KEY);
    let voting_key = bls_public_key(VALIDATOR_VOTING_KEY);

    // Works in the valid case.
    let tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::DeactivateValidator {
            validator_address: validator_address.clone(),
            proof: SignatureProof::default(),
        },
        0,
        &signing_keypair,
    );

    let mut tx_logger = TransactionLog::empty();
    let receipt = staking_contract
        .commit_incoming_transaction(
            &tx,
            &block_state,
            data_store.write(&mut db_txn),
            &mut tx_logger,
        )
        .expect("Failed to commit transaction");

    assert_eq!(receipt, None);
    assert_eq!(
        tx_logger.logs,
        vec![Log::DeactivateValidator {
            validator_address: validator_address.clone()
        }]
    );

    let validator = staking_contract
        .get_validator(&data_store.read(&db_txn), &validator_address)
        .expect("Validator should exist");

    assert_eq!(validator.address, validator_address);
    assert_eq!(validator.signing_key, signing_key);
    assert_eq!(validator.voting_key, voting_key);
    assert_eq!(validator.reward_address, validator_address);
    assert_eq!(validator.signal_data, None);
    assert_eq!(
        validator.total_stake,
        Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT + 150_000_000)
    );
    assert_eq!(validator.num_stakers, 1);
    assert_eq!(validator.inactive_since, Some(2));

    assert!(!staking_contract
        .active_validators
        .contains_key(&validator_address));

    // Try with an already inactive validator.
    assert_eq!(
        staking_contract.commit_incoming_transaction(
            &tx,
            &block_state,
            data_store.write(&mut db_txn),
            &mut TransactionLog::empty()
        ),
        Err(AccountError::InvalidForRecipient)
    );

    // Revert the transaction.
    let mut tx_logger = TransactionLog::empty();
    staking_contract
        .revert_incoming_transaction(
            &tx,
            &block_state,
            receipt,
            data_store.write(&mut db_txn),
            &mut tx_logger,
        )
        .expect("Failed to revert transaction");

    assert_eq!(
        tx_logger.logs,
        vec![Log::DeactivateValidator {
            validator_address: validator_address.clone()
        }]
    );

    let validator = staking_contract
        .get_validator(&data_store.read(&db_txn), &validator_address)
        .expect("Validator should exist");

    assert_eq!(validator.address, validator_address);
    assert_eq!(validator.signing_key, signing_key);
    assert_eq!(validator.voting_key, voting_key);
    assert_eq!(validator.reward_address, validator_address);
    assert_eq!(validator.signal_data, None);
    assert_eq!(
        validator.total_stake,
        Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT + 150_000_000)
    );
    assert_eq!(validator.num_stakers, 1);
    assert_eq!(validator.inactive_since, None);

    assert!(staking_contract
        .active_validators
        .contains_key(&validator_address));

    // Try with a non-existent validator.
    let fake_address = staker_address();

    let tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::DeactivateValidator {
            validator_address: fake_address.clone(),
            proof: SignatureProof::default(),
        },
        0,
        &signing_keypair,
    );

    assert_eq!(
        staking_contract.commit_incoming_transaction(
            &tx,
            &block_state,
            data_store.write(&mut db_txn),
            &mut TransactionLog::empty()
        ),
        Err(AccountError::NonExistentAddress {
            address: fake_address
        })
    );

    // Try with a wrong signature.
    let invalid_tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::DeactivateValidator {
            validator_address: validator_address.clone(),
            proof: SignatureProof::default(),
        },
        0,
        &cold_keypair,
    );

    assert_eq!(
        staking_contract.commit_incoming_transaction(
            &invalid_tx,
            &block_state,
            data_store.write(&mut db_txn),
            &mut TransactionLog::empty()
        ),
        Err(AccountError::InvalidSignature)
    );
}

#[test]
fn reactivate_validator_works() {
    let env = VolatileDatabase::new(10).unwrap();
    let accounts = Accounts::new(env.clone());
    let data_store = accounts.data_store(&Policy::STAKING_CONTRACT_ADDRESS);
    let block_state = BlockState::new(2, 2);
    let mut db_txn = env.write_transaction();

    let mut staking_contract = make_sample_contract(data_store.write(&mut db_txn), true);

    let validator_address = validator_address();
    let cold_keypair = ed25519_key_pair(VALIDATOR_PRIVATE_KEY);
    let signing_key = ed25519_public_key(VALIDATOR_SIGNING_KEY);
    let signing_keypair = ed25519_key_pair(VALIDATOR_SIGNING_SECRET_KEY);
    let voting_key = bls_public_key(VALIDATOR_VOTING_KEY);

    // To begin with, deactivate the validator.
    let tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::DeactivateValidator {
            validator_address: validator_address.clone(),
            proof: SignatureProof::default(),
        },
        0,
        &signing_keypair,
    );

    staking_contract
        .commit_incoming_transaction(
            &tx,
            &block_state,
            data_store.write(&mut db_txn),
            &mut TransactionLog::empty(),
        )
        .expect("Failed to commit transaction");

    // Works in the valid case.
    let tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::ReactivateValidator {
            validator_address: validator_address.clone(),
            proof: SignatureProof::default(),
        },
        0,
        &signing_keypair,
    );

    let mut tx_logger = TransactionLog::empty();
    let receipt = staking_contract
        .commit_incoming_transaction(
            &tx,
            &block_state,
            data_store.write(&mut db_txn),
            &mut tx_logger,
        )
        .expect("Failed to commit transaction");

    let expected_receipt = ReactivateValidatorReceipt {
        was_inactive_since: 2,
        current_epoch_disabled_slots: None,
    };

    assert_eq!(receipt, Some(expected_receipt.into()));
    assert_eq!(
        tx_logger.logs,
        vec![Log::ReactivateValidator {
            validator_address: validator_address.clone()
        }]
    );

    let validator = staking_contract
        .get_validator(&data_store.read(&db_txn), &validator_address)
        .expect("Validator should exist");

    assert_eq!(validator.address, validator_address);
    assert_eq!(validator.signing_key, signing_key);
    assert_eq!(validator.voting_key, voting_key);
    assert_eq!(validator.reward_address, validator_address);
    assert_eq!(validator.signal_data, None);
    assert_eq!(
        validator.total_stake,
        Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT + 150_000_000)
    );
    assert_eq!(validator.num_stakers, 1);
    assert_eq!(validator.inactive_since, None);

    assert_eq!(
        staking_contract.active_validators.get(&validator_address),
        Some(&Coin::from_u64_unchecked(
            Policy::VALIDATOR_DEPOSIT + 150_000_000
        ))
    );

    // Reactivate a slashed validator.
    // Slash the validator slot.
    let inherent = Inherent::Slash {
        slot: SlashedSlot {
            slot: 1,
            validator_address: validator_address.clone(),
            event_block: Policy::blocks_per_epoch() - 1,
        },
    };
    let mut logs = vec![];
    let mut inherent_logger = InherentLogger::new(&mut logs);
    let receipt = staking_contract
        .commit_inherent(
            &inherent,
            &block_state,
            data_store.write(&mut db_txn),
            &mut inherent_logger,
        )
        .expect("Failed to commit inherent");
    assert_eq!(
        receipt,
        Some(
            SlashReceipt {
                newly_deactivated: true,
                newly_disabled: true,
                newly_lost_rewards: true
            }
            .into()
        )
    );
    assert_eq!(
        logs,
        vec![
            Log::Slash {
                validator_address: validator_address.clone(),
                event_block: Policy::blocks_per_epoch() - 1,
                slot: 1,
                newly_disabled: true,
                newly_deactivated: true
            },
            Log::DeactivateValidator {
                validator_address: validator_address.clone(),
            }
        ]
    );

    assert!(!staking_contract
        .active_validators
        .contains_key(&validator_address));

    let mut tx_logger = TransactionLog::empty();
    let receipt = staking_contract
        .commit_incoming_transaction(
            &tx,
            &block_state,
            data_store.write(&mut db_txn),
            &mut tx_logger,
        )
        .expect("Failed to commit transaction");

    let mut bitset = BTreeSet::new();
    bitset.insert(1);
    let expected_receipt = ReactivateValidatorReceipt {
        was_inactive_since: 2,
        current_epoch_disabled_slots: Some(bitset),
    };

    assert_eq!(receipt, Some(expected_receipt.into()));
    assert_eq!(
        tx_logger.logs,
        vec![Log::ReactivateValidator {
            validator_address: validator_address.clone()
        }]
    );

    let validator = staking_contract
        .get_validator(&data_store.read(&db_txn), &validator_address)
        .expect("Validator should exist");

    assert_eq!(validator.address, validator_address);
    assert_eq!(validator.signing_key, signing_key);
    assert_eq!(validator.voting_key, voting_key);
    assert_eq!(validator.reward_address, validator_address);
    assert_eq!(validator.signal_data, None);
    assert_eq!(
        validator.total_stake,
        Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT + 150_000_000)
    );
    assert_eq!(validator.num_stakers, 1);
    assert_eq!(validator.inactive_since, None);

    assert_eq!(
        staking_contract.active_validators.get(&validator_address),
        Some(&Coin::from_u64_unchecked(
            Policy::VALIDATOR_DEPOSIT + 150_000_000
        ))
    );

    // Try with an already active validator.
    assert_eq!(
        staking_contract.commit_incoming_transaction(
            &tx,
            &block_state,
            data_store.write(&mut db_txn),
            &mut TransactionLog::empty()
        ),
        Err(AccountError::InvalidForRecipient)
    );

    // Revert the transaction.
    let mut tx_logger = TransactionLog::empty();
    staking_contract
        .revert_incoming_transaction(
            &tx,
            &block_state,
            receipt,
            data_store.write(&mut db_txn),
            &mut tx_logger,
        )
        .expect("Failed to revert transaction");

    assert_eq!(
        tx_logger.logs,
        vec![Log::ReactivateValidator {
            validator_address: validator_address.clone()
        }]
    );

    let validator = staking_contract
        .get_validator(&data_store.read(&db_txn), &validator_address)
        .expect("Validator should exist");

    assert_eq!(validator.address, validator_address);
    assert_eq!(validator.signing_key, signing_key);
    assert_eq!(validator.voting_key, voting_key);
    assert_eq!(validator.reward_address, validator_address);
    assert_eq!(validator.signal_data, None);
    assert_eq!(
        validator.total_stake,
        Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT + 150_000_000)
    );
    assert_eq!(validator.num_stakers, 1);
    assert_eq!(validator.inactive_since, Some(2));

    assert!(!staking_contract
        .active_validators
        .contains_key(&validator_address));

    // Try with a non-existent validator.
    let fake_address = staker_address();

    let invalid_tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::ReactivateValidator {
            validator_address: fake_address.clone(),
            proof: SignatureProof::default(),
        },
        0,
        &signing_keypair,
    );

    assert_eq!(
        staking_contract.commit_incoming_transaction(
            &invalid_tx,
            &block_state,
            data_store.write(&mut db_txn),
            &mut TransactionLog::empty()
        ),
        Err(AccountError::NonExistentAddress {
            address: fake_address
        })
    );

    // Try with a wrong signature.
    let invalid_tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::ReactivateValidator {
            validator_address: validator_address.clone(),
            proof: SignatureProof::default(),
        },
        0,
        &cold_keypair,
    );

    assert_eq!(
        staking_contract.commit_incoming_transaction(
            &invalid_tx,
            &block_state,
            data_store.write(&mut db_txn),
            &mut TransactionLog::empty()
        ),
        Err(AccountError::InvalidSignature)
    );

    // Try with a retired validator.
    let retire_tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::RetireValidator {
            proof: SignatureProof::default(),
        },
        0,
        &cold_keypair,
    );

    staking_contract
        .commit_incoming_transaction(
            &retire_tx,
            &block_state,
            data_store.write(&mut db_txn),
            &mut TransactionLog::empty(),
        )
        .expect("Failed to commit transaction");

    assert_eq!(
        staking_contract.commit_incoming_transaction(
            &tx,
            &block_state,
            data_store.write(&mut db_txn),
            &mut TransactionLog::empty()
        ),
        Err(AccountError::InvalidForRecipient)
    );
}

#[test]
fn retire_validator_works() {
    let env = VolatileDatabase::new(10).unwrap();
    let accounts = Accounts::new(env.clone());
    let data_store = accounts.data_store(&Policy::STAKING_CONTRACT_ADDRESS);
    let block_state = BlockState::new(2, 2);
    let mut db_txn = env.write_transaction();

    let mut staking_contract = make_sample_contract(data_store.write(&mut db_txn), true);

    let validator_address = validator_address();
    let cold_keypair = ed25519_key_pair(VALIDATOR_PRIVATE_KEY);
    let signing_keypair = ed25519_key_pair(VALIDATOR_SIGNING_SECRET_KEY);

    // Works in the valid case.
    let tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::RetireValidator {
            proof: SignatureProof::default(),
        },
        0,
        &cold_keypair,
    );

    let receipt = staking_contract
        .commit_incoming_transaction(
            &tx,
            &block_state,
            data_store.write(&mut db_txn),
            &mut TransactionLog::empty(),
        )
        .expect("Failed to commit transaction");

    let expected_receipt = RetireValidatorReceipt { was_active: true };
    assert_eq!(receipt, Some(expected_receipt.into()));

    assert!(!staking_contract
        .current_epoch_disabled_slots
        .contains_key(&validator_address));
    assert!(!staking_contract
        .previous_epoch_disabled_slots
        .contains_key(&validator_address));

    // Try with an already retired validator.
    assert_eq!(
        staking_contract.commit_incoming_transaction(
            &tx,
            &block_state,
            data_store.write(&mut db_txn),
            &mut TransactionLog::empty()
        ),
        Err(AccountError::InvalidForRecipient)
    );

    let mut tx_logger = TransactionLog::empty();
    staking_contract
        .revert_incoming_transaction(
            &tx,
            &block_state,
            receipt,
            data_store.write(&mut db_txn),
            &mut tx_logger,
        )
        .expect("Failed to revert transaction");

    assert_eq!(
        tx_logger.logs,
        vec![
            Log::DeactivateValidator {
                validator_address: validator_address.clone()
            },
            Log::RetireValidator {
                validator_address: validator_address.clone()
            }
        ]
    );

    assert!(staking_contract
        .active_validators
        .contains_key(&validator_address));

    // Try with a wrong signature.
    let invalid_tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::RetireValidator {
            proof: SignatureProof::default(),
        },
        0,
        &signing_keypair,
    );

    assert_eq!(
        staking_contract.commit_incoming_transaction(
            &invalid_tx,
            &block_state,
            data_store.write(&mut db_txn),
            &mut TransactionLog::empty()
        ),
        Err(AccountError::NonExistentAddress {
            address: Address::from(&signing_keypair.public)
        })
    );
}

#[test]
fn delete_validator_works() {
    let env = VolatileDatabase::new(10).unwrap();
    let accounts = Accounts::new(env.clone());
    let data_store = accounts.data_store(&Policy::STAKING_CONTRACT_ADDRESS);
    let block_state = BlockState::new(2, 2);
    let mut db_txn = env.write_transaction();

    let mut staking_contract = make_sample_contract(data_store.write(&mut db_txn), true);

    // Doesn't work when the validator is still active.
    let tx = make_delete_validator_transaction();

    assert_eq!(
        staking_contract.commit_outgoing_transaction(
            &tx,
            &block_state,
            data_store.write(&mut db_txn),
            &mut TransactionLog::empty()
        ),
        Err(AccountError::InvalidForSender)
    );

    // Deactivate validator.
    let deactivate_tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::DeactivateValidator {
            validator_address: validator_address(),
            proof: SignatureProof::default(),
        },
        0,
        &ed25519_key_pair(VALIDATOR_SIGNING_SECRET_KEY),
    );

    staking_contract
        .commit_incoming_transaction(
            &deactivate_tx,
            &block_state,
            data_store.write(&mut db_txn),
            &mut TransactionLog::empty(),
        )
        .expect("Failed to commit transaction");

    // Doesn't work with a deactivated but not retired validator.
    let after_cooldown = Policy::election_block_after(2) + Policy::blocks_per_batch() + 1;
    let block_state = BlockState::new(after_cooldown, 1000);

    assert_eq!(
        staking_contract.commit_outgoing_transaction(
            &tx,
            &block_state,
            data_store.write(&mut db_txn),
            &mut TransactionLog::empty()
        ),
        Err(AccountError::InvalidForSender)
    );

    // Retire the validator.
    let retire_tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::RetireValidator {
            proof: SignatureProof::default(),
        },
        0,
        &ed25519_key_pair(VALIDATOR_PRIVATE_KEY),
    );

    let block_state = BlockState::new(3, 3);
    staking_contract
        .commit_incoming_transaction(
            &retire_tx,
            &block_state,
            data_store.write(&mut db_txn),
            &mut TransactionLog::empty(),
        )
        .expect("Failed to commit transaction");

    // Doesn't work if the cooldown hasn't expired.
    assert_eq!(
        staking_contract.commit_outgoing_transaction(
            &tx,
            &BlockState::new(after_cooldown - 1, 999),
            data_store.write(&mut db_txn),
            &mut TransactionLog::empty()
        ),
        Err(AccountError::InvalidForSender)
    );

    // Works in the valid case.
    let validator_address = validator_address();
    let signing_key = ed25519_public_key(VALIDATOR_SIGNING_KEY);
    let voting_key = bls_public_key(VALIDATOR_VOTING_KEY);
    let reward_address = validator_address.clone();
    let staker_address = staker_address();

    let block_state = BlockState::new(after_cooldown, 1000);

    let mut tx_logger = TransactionLog::empty();
    let receipt = staking_contract
        .commit_outgoing_transaction(
            &tx,
            &block_state,
            data_store.write(&mut db_txn),
            &mut tx_logger,
        )
        .expect("Failed to commit transaction");

    let expected_receipt = DeleteValidatorReceipt {
        signing_key,
        voting_key: voting_key.clone(),
        reward_address: reward_address.clone(),
        signal_data: None,
        inactive_since: 2,
    };
    assert_eq!(receipt, Some(expected_receipt.into()));

    assert_eq!(
        tx_logger.logs,
        vec![
            Log::PayFee {
                from: tx.sender.clone(),
                fee: tx.fee,
            },
            Log::Transfer {
                from: tx.sender.clone(),
                to: tx.recipient.clone(),
                amount: tx.value,
                data: None,
            },
            Log::DeleteValidator {
                validator_address: validator_address.clone(),
                reward_address: reward_address.clone(),
            }
        ]
    );

    assert_eq!(
        staking_contract.get_validator(&data_store.read(&db_txn), &validator_address),
        None
    );

    assert_eq!(
        staking_contract.get_tombstone(&data_store.read(&db_txn), &validator_address),
        Some(Tombstone {
            remaining_stake: Coin::from_u64_unchecked(150_000_000),
            num_remaining_stakers: 1,
        })
    );

    let staker = staking_contract
        .get_staker(&data_store.read(&db_txn), &staker_address)
        .expect("Staker should exist");

    assert_eq!(staker.delegation, Some(validator_address.clone()));

    assert_eq!(
        staking_contract.balance,
        Coin::from_u64_unchecked(150_000_000)
    );

    // Remove the staker.
    let unstake_tx = make_unstake_transaction(150_000_000);

    let unstake_block_state = BlockState::new(block_state.number + 1, block_state.time + 1);

    let unstake_receipt = staking_contract
        .commit_outgoing_transaction(
            &unstake_tx,
            &unstake_block_state,
            data_store.write(&mut db_txn),
            &mut TransactionLog::empty(),
        )
        .expect("Failed to commit transaction");

    let expected_receipt = StakerReceipt {
        delegation: Some(validator_address.clone()),
    };
    assert_eq!(unstake_receipt, Some(expected_receipt.into()));

    assert_eq!(
        staking_contract.get_staker(&data_store.read(&db_txn), &staker_address),
        None
    );

    assert_eq!(
        staking_contract.get_tombstone(&data_store.read(&db_txn), &validator_address),
        None
    );

    assert_eq!(staking_contract.balance, Coin::ZERO);

    // Revert the unstake transaction.
    staking_contract
        .revert_outgoing_transaction(
            &unstake_tx,
            &unstake_block_state,
            unstake_receipt,
            data_store.write(&mut db_txn),
            &mut TransactionLog::empty(),
        )
        .expect("Failed to revert transaction");

    assert_eq!(
        staking_contract.get_tombstone(&data_store.read(&db_txn), &validator_address),
        Some(Tombstone {
            remaining_stake: Coin::from_u64_unchecked(150_000_000),
            num_remaining_stakers: 1,
        })
    );

    let staker = staking_contract
        .get_staker(&data_store.read(&db_txn), &staker_address)
        .expect("Staker should exist");

    assert_eq!(staker.delegation, Some(validator_address.clone()));

    assert_eq!(
        staking_contract.balance,
        Coin::from_u64_unchecked(150_000_000)
    );

    // Revert the delete transaction.
    let mut tx_logger = TransactionLog::empty();
    staking_contract
        .revert_outgoing_transaction(
            &tx,
            &block_state,
            receipt,
            data_store.write(&mut db_txn),
            &mut tx_logger,
        )
        .expect("Failed to revert transaction");

    assert_eq!(
        tx_logger.logs,
        vec![
            Log::DeleteValidator {
                validator_address: validator_address.clone(),
                reward_address: reward_address.clone(),
            },
            Log::Transfer {
                from: tx.sender.clone(),
                to: tx.recipient.clone(),
                amount: tx.value,
                data: None,
            },
            Log::PayFee {
                from: tx.sender.clone(),
                fee: tx.fee,
            },
        ]
    );

    let validator = staking_contract
        .get_validator(&data_store.read(&db_txn), &validator_address)
        .expect("Validator should exist");

    assert_eq!(validator.address, validator_address);
    assert_eq!(validator.signing_key, signing_key);
    assert_eq!(validator.voting_key, voting_key);
    assert_eq!(validator.reward_address, reward_address);
    assert_eq!(validator.signal_data, None);
    assert_eq!(
        validator.total_stake,
        Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT + 150_000_000)
    );
    assert_eq!(validator.num_stakers, 1);
    assert_eq!(validator.inactive_since, Some(2));
    assert_eq!(validator.retired, true);

    assert_eq!(
        staking_contract.get_tombstone(&data_store.read(&db_txn), &validator_address),
        None
    );

    assert_eq!(
        staking_contract.balance,
        Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT + 150_000_000)
    );
}

#[test]
fn create_staker_works() {
    let env = VolatileDatabase::new(10).unwrap();
    let accounts = Accounts::new(env.clone());
    let data_store = accounts.data_store(&Policy::STAKING_CONTRACT_ADDRESS);
    let block_state = BlockState::new(2, 2);
    let mut db_txn = env.write_transaction();

    let mut staking_contract = make_sample_contract(data_store.write(&mut db_txn), false);

    let staker_keypair = ed25519_key_pair(STAKER_PRIVATE_KEY);
    let staker_address = staker_address();
    let validator_address = validator_address();

    // Works in the valid case.
    let tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::CreateStaker {
            delegation: Some(validator_address.clone()),
            proof: SignatureProof::default(),
        },
        150_000_000,
        &staker_keypair,
    );

    let mut tx_logger = TransactionLog::empty();
    let receipt = staking_contract
        .commit_incoming_transaction(
            &tx,
            &block_state,
            data_store.write(&mut db_txn),
            &mut tx_logger,
        )
        .expect("Failed to commit transaction");

    assert_eq!(receipt, None);
    assert_eq!(
        tx_logger.logs,
        vec![Log::CreateStaker {
            staker_address: staker_address.clone(),
            validator_address: Some(validator_address.clone()),
            value: tx.value,
        }]
    );

    let staker = staking_contract
        .get_staker(&data_store.read(&db_txn), &staker_address)
        .expect("Staker should exist");

    assert_eq!(staker.address, staker_address);
    assert_eq!(staker.balance, Coin::from_u64_unchecked(150_000_000));
    assert_eq!(staker.delegation, Some(validator_address.clone()));

    let validator = staking_contract
        .get_validator(&data_store.read(&db_txn), &validator_address)
        .expect("Validator should exist");

    assert_eq!(
        validator.total_stake,
        Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT + 150_000_000)
    );
    assert_eq!(validator.num_stakers, 1);

    assert_eq!(
        staking_contract.balance,
        Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT + 150_000_000)
    );

    assert_eq!(
        staking_contract.active_validators.get(&validator_address),
        Some(&Coin::from_u64_unchecked(
            Policy::VALIDATOR_DEPOSIT + 150_000_000
        ))
    );

    // Doesn't work when the staker already exists.
    assert_eq!(
        staking_contract.commit_incoming_transaction(
            &tx,
            &block_state,
            data_store.write(&mut db_txn),
            &mut TransactionLog::empty()
        ),
        Err(AccountError::AlreadyExistentAddress {
            address: staker_address.clone()
        })
    );

    // Revert the transaction.
    let mut tx_logger = TransactionLog::empty();
    staking_contract
        .revert_incoming_transaction(
            &tx,
            &block_state,
            None,
            data_store.write(&mut db_txn),
            &mut tx_logger,
        )
        .expect("Failed to revert transaction");

    assert_eq!(
        tx_logger.logs,
        vec![Log::CreateStaker {
            staker_address: staker_address.clone(),
            validator_address: Some(validator_address.clone()),
            value: tx.value,
        }]
    );

    assert_eq!(
        staking_contract.get_staker(&data_store.read(&db_txn), &staker_address),
        None
    );

    let validator = staking_contract
        .get_validator(&data_store.read(&db_txn), &validator_address)
        .expect("Validator should exist");

    assert_eq!(
        validator.total_stake,
        Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT)
    );
    assert_eq!(validator.num_stakers, 0);

    assert_eq!(
        staking_contract.balance,
        Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT)
    );

    assert_eq!(
        staking_contract.active_validators.get(&validator_address),
        Some(&Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT))
    );
}

#[test]
fn stake_works() {
    let env = VolatileDatabase::new(10).unwrap();
    let accounts = Accounts::new(env.clone());
    let data_store = accounts.data_store(&Policy::STAKING_CONTRACT_ADDRESS);
    let block_state = BlockState::new(2, 2);
    let mut db_txn = env.write_transaction();

    let mut staking_contract = make_sample_contract(data_store.write(&mut db_txn), true);

    let staker_keypair = ed25519_key_pair(STAKER_PRIVATE_KEY);
    let staker_address = staker_address();
    let validator_address = validator_address();

    // Works in the valid case.
    let tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::AddStake {
            staker_address: staker_address.clone(),
        },
        150_000_000,
        &staker_keypair,
    );

    let mut tx_logger = TransactionLog::empty();
    let receipt = staking_contract
        .commit_incoming_transaction(
            &tx,
            &block_state,
            data_store.write(&mut db_txn),
            &mut tx_logger,
        )
        .expect("Failed to commit transaction");

    assert_eq!(receipt, None);
    assert_eq!(
        tx_logger.logs,
        vec![Log::Stake {
            staker_address: staker_address.clone(),
            validator_address: Some(validator_address.clone()),
            value: tx.value,
        }]
    );

    let staker = staking_contract
        .get_staker(&data_store.read(&db_txn), &staker_address)
        .expect("Staker should exist");

    assert_eq!(staker.address, staker_address);
    assert_eq!(staker.balance, Coin::from_u64_unchecked(300_000_000));
    assert_eq!(staker.delegation, Some(validator_address.clone()));

    let validator = staking_contract
        .get_validator(&data_store.read(&db_txn), &validator_address)
        .expect("Validator should exist");

    assert_eq!(
        validator.total_stake,
        Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT + 300_000_000)
    );

    assert_eq!(
        staking_contract.balance,
        Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT + 300_000_000)
    );

    assert_eq!(
        staking_contract.active_validators.get(&validator_address),
        Some(&Coin::from_u64_unchecked(
            Policy::VALIDATOR_DEPOSIT + 300_000_000
        ))
    );

    // Revert the transaction.
    let mut tx_logger = TransactionLog::empty();
    staking_contract
        .revert_incoming_transaction(
            &tx,
            &block_state,
            None,
            data_store.write(&mut db_txn),
            &mut tx_logger,
        )
        .expect("Failed to revert transaction");

    assert_eq!(
        tx_logger.logs,
        vec![Log::Stake {
            staker_address: staker_address.clone(),
            validator_address: Some(validator_address.clone()),
            value: tx.value,
        }]
    );

    let staker = staking_contract
        .get_staker(&data_store.read(&db_txn), &staker_address)
        .expect("Staker should exist");

    assert_eq!(staker.address, staker_address);
    assert_eq!(staker.balance, Coin::from_u64_unchecked(150_000_000));
    assert_eq!(staker.delegation, Some(validator_address.clone()));

    let validator = staking_contract
        .get_validator(&data_store.read(&db_txn), &validator_address)
        .expect("Validator should exist");

    assert_eq!(
        validator.total_stake,
        Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT + 150_000_000)
    );

    assert_eq!(
        staking_contract.balance,
        Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT + 150_000_000)
    );

    assert_eq!(
        staking_contract.active_validators.get(&validator_address),
        Some(&Coin::from_u64_unchecked(
            Policy::VALIDATOR_DEPOSIT + 150_000_000
        ))
    );
}

#[test]
fn update_staker_works() {
    let env = VolatileDatabase::new(10).unwrap();
    let accounts = Accounts::new(env.clone());
    let data_store = accounts.data_store(&Policy::STAKING_CONTRACT_ADDRESS);
    let block_state = BlockState::new(2, 2);
    let mut db_txn = env.write_transaction();

    let mut staking_contract = make_sample_contract(data_store.write(&mut db_txn), true);

    let staker_keypair = ed25519_key_pair(STAKER_PRIVATE_KEY);
    let staker_address = staker_address();
    let validator_address = validator_address();
    let other_validator_address = Address::from([69u8; 20]);
    let signing_key = ed25519_public_key(VALIDATOR_SIGNING_KEY);
    let voting_key = bls_public_key(VALIDATOR_VOTING_KEY);

    // To begin with, add another validator.
    let mut data_store_write = data_store.write(&mut db_txn);
    let mut store = StakingContractStoreWrite::new(&mut data_store_write);

    staking_contract
        .create_validator(
            &mut store,
            &other_validator_address,
            signing_key,
            voting_key,
            other_validator_address.clone(),
            None,
            Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT),
            &mut TransactionLog::empty(),
        )
        .expect("Failed to create validator");

    // Works when changing to another validator.
    let tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::UpdateStaker {
            new_delegation: Some(other_validator_address.clone()),
            proof: SignatureProof::default(),
        },
        0,
        &staker_keypair,
    );

    let mut tx_logger = TransactionLog::empty();
    let receipt = staking_contract
        .commit_incoming_transaction(
            &tx,
            &block_state,
            data_store.write(&mut db_txn),
            &mut tx_logger,
        )
        .expect("Failed to commit transaction");

    let expected_receipt = StakerReceipt {
        delegation: Some(validator_address.clone()),
    };
    assert_eq!(receipt, Some(expected_receipt.into()));

    assert_eq!(
        tx_logger.logs,
        vec![Log::UpdateStaker {
            staker_address: staker_address.clone(),
            old_validator_address: Some(validator_address.clone()),
            new_validator_address: Some(other_validator_address.clone()),
        }]
    );

    let staker = staking_contract
        .get_staker(&data_store.read(&db_txn), &staker_address)
        .expect("Staker should exist");

    assert_eq!(staker.address, staker_address);
    assert_eq!(staker.balance, Coin::from_u64_unchecked(150_000_000));
    assert_eq!(staker.delegation, Some(other_validator_address.clone()));

    let old_validator = staking_contract
        .get_validator(&data_store.read(&db_txn), &validator_address)
        .expect("Validator should exist");

    assert_eq!(
        old_validator.total_stake,
        Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT)
    );
    assert_eq!(old_validator.num_stakers, 0);

    let new_validator = staking_contract
        .get_validator(&data_store.read(&db_txn), &other_validator_address)
        .expect("Validator should exist");

    assert_eq!(
        new_validator.total_stake,
        Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT + 150_000_000)
    );
    assert_eq!(new_validator.num_stakers, 1);

    assert_eq!(
        staking_contract.active_validators.get(&validator_address),
        Some(&Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT))
    );

    assert_eq!(
        staking_contract
            .active_validators
            .get(&other_validator_address),
        Some(&Coin::from_u64_unchecked(
            Policy::VALIDATOR_DEPOSIT + 150_000_000
        ))
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

    assert_eq!(
        staking_contract.commit_incoming_transaction(
            &tx,
            &block_state,
            data_store.write(&mut db_txn),
            &mut TransactionLog::empty()
        ),
        Err(AccountError::NonExistentAddress {
            address: staker_address.clone()
        })
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

    let mut tx_logger = TransactionLog::empty();
    let receipt = staking_contract
        .commit_incoming_transaction(
            &tx,
            &block_state,
            data_store.write(&mut db_txn),
            &mut tx_logger,
        )
        .expect("Failed to commit transaction");

    let expected_receipt = StakerReceipt {
        delegation: Some(other_validator_address.clone()),
    };
    assert_eq!(receipt, Some(expected_receipt.into()));

    assert_eq!(
        tx_logger.logs,
        vec![Log::UpdateStaker {
            staker_address: staker_address.clone(),
            old_validator_address: Some(other_validator_address.clone()),
            new_validator_address: None,
        }]
    );

    let staker = staking_contract
        .get_staker(&data_store.read(&db_txn), &staker_address)
        .expect("Staker should exist");

    assert_eq!(staker.address, staker_address);
    assert_eq!(staker.balance, Coin::from_u64_unchecked(150_000_000));
    assert_eq!(staker.delegation, None);

    let other_validator = staking_contract
        .get_validator(&data_store.read(&db_txn), &other_validator_address)
        .expect("Validator should exist");

    assert_eq!(
        other_validator.total_stake,
        Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT)
    );
    assert_eq!(other_validator.num_stakers, 0);

    assert_eq!(
        staking_contract
            .active_validators
            .get(&other_validator_address),
        Some(&Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT))
    );

    // Revert the transaction.
    let mut tx_logger = TransactionLog::empty();
    staking_contract
        .revert_incoming_transaction(
            &tx,
            &block_state,
            receipt,
            data_store.write(&mut db_txn),
            &mut tx_logger,
        )
        .expect("Failed to revert transaction");

    assert_eq!(
        tx_logger.logs,
        vec![Log::UpdateStaker {
            staker_address: staker_address.clone(),
            old_validator_address: Some(other_validator_address.clone()),
            new_validator_address: None,
        }]
    );

    let validator = staking_contract
        .get_validator(&data_store.read(&db_txn), &other_validator_address)
        .expect("Validator should exist");

    assert_eq!(
        validator.total_stake,
        Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT + 150_000_000)
    );
    assert_eq!(validator.num_stakers, 1);

    assert_eq!(
        staking_contract
            .active_validators
            .get(&other_validator_address),
        Some(&Coin::from_u64_unchecked(
            Policy::VALIDATOR_DEPOSIT + 150_000_000
        ))
    );

    // Doesn't work when the staker doesn't exist.
    let fake_keypair = ed25519_key_pair(VALIDATOR_PRIVATE_KEY);

    let tx = make_signed_incoming_transaction(
        IncomingStakingTransactionData::UpdateStaker {
            new_delegation: None,
            proof: SignatureProof::default(),
        },
        0,
        &fake_keypair,
    );

    assert_eq!(
        staking_contract.commit_incoming_transaction(
            &tx,
            &block_state,
            data_store.write(&mut db_txn),
            &mut TransactionLog::empty()
        ),
        Err(AccountError::NonExistentAddress {
            address: (&fake_keypair.public).into()
        })
    );
}

#[test]
fn unstake_works() {
    let env = VolatileDatabase::new(10).unwrap();
    let accounts = Accounts::new(env.clone());
    let data_store = accounts.data_store(&Policy::STAKING_CONTRACT_ADDRESS);
    let block_state = BlockState::new(2, 2);
    let mut db_txn = env.write_transaction();

    let mut staking_contract = make_sample_contract(data_store.write(&mut db_txn), true);

    let staker_address = staker_address();
    let validator_address = validator_address();

    // Doesn't work if the value is greater than the balance.
    let tx = make_unstake_transaction(200_000_000);

    assert_eq!(
        staking_contract.commit_outgoing_transaction(
            &tx,
            &block_state,
            data_store.write(&mut db_txn),
            &mut TransactionLog::empty()
        ),
        Err(AccountError::InsufficientFunds {
            needed: Coin::from_u64_unchecked(200_000_000),
            balance: Coin::from_u64_unchecked(150_000_000)
        })
    );

    // Works in the valid case.
    let tx = make_unstake_transaction(100_000_000);

    let mut tx_logger = TransactionLog::empty();
    let receipt = staking_contract
        .commit_outgoing_transaction(
            &tx,
            &block_state,
            data_store.write(&mut db_txn),
            &mut tx_logger,
        )
        .expect("Failed to commit transaction");

    assert_eq!(receipt, None);
    assert_eq!(
        tx_logger.logs,
        vec![
            Log::PayFee {
                from: tx.sender.clone(),
                fee: tx.fee,
            },
            Log::Transfer {
                from: tx.sender.clone(),
                to: tx.recipient.clone(),
                amount: tx.value,
                data: None,
            },
            Log::Unstake {
                staker_address: staker_address.clone(),
                validator_address: Some(validator_address.clone()),
                value: Coin::from_u64_unchecked(100_000_000),
            }
        ]
    );

    let staker = staking_contract
        .get_staker(&data_store.read(&db_txn), &staker_address)
        .expect("Staker should exist");

    assert_eq!(staker.address, staker_address);
    assert_eq!(staker.balance, Coin::from_u64_unchecked(50_000_000));
    assert_eq!(staker.delegation, Some(validator_address.clone()));

    let validator = staking_contract
        .get_validator(&data_store.read(&db_txn), &validator_address)
        .expect("Validator should exist");

    assert_eq!(
        validator.total_stake,
        Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT + 50_000_000)
    );
    assert_eq!(validator.num_stakers, 1);

    assert_eq!(
        staking_contract.balance,
        Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT + 50_000_000)
    );

    assert_eq!(
        staking_contract.active_validators.get(&validator_address),
        Some(&Coin::from_u64_unchecked(
            Policy::VALIDATOR_DEPOSIT + 50_000_000
        ))
    );

    // Works when removing the entire balance.
    let tx = make_unstake_transaction(50_000_000);

    let block_state = BlockState::new(3, 3);

    let mut tx_logger = TransactionLog::empty();
    let receipt = staking_contract
        .commit_outgoing_transaction(
            &tx,
            &block_state,
            data_store.write(&mut db_txn),
            &mut tx_logger,
        )
        .expect("Failed to commit transaction");

    let expected_receipt = StakerReceipt {
        delegation: Some(validator_address.clone()),
    };
    assert_eq!(receipt, Some(expected_receipt.into()));

    assert_eq!(
        tx_logger.logs,
        vec![
            Log::PayFee {
                from: tx.sender.clone(),
                fee: tx.fee,
            },
            Log::Transfer {
                from: tx.sender.clone(),
                to: tx.recipient.clone(),
                amount: tx.value,
                data: None,
            },
            Log::Unstake {
                staker_address: staker_address.clone(),
                validator_address: Some(validator_address.clone()),
                value: Coin::from_u64_unchecked(50_000_000),
            }
        ]
    );

    assert_eq!(
        staking_contract.get_staker(&data_store.read(&db_txn), &staker_address),
        None
    );

    let validator = staking_contract
        .get_validator(&data_store.read(&db_txn), &validator_address)
        .expect("Validator should exist");

    assert_eq!(
        validator.total_stake,
        Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT)
    );
    assert_eq!(validator.num_stakers, 0);

    assert_eq!(
        staking_contract.balance,
        Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT)
    );

    assert_eq!(
        staking_contract.active_validators.get(&validator_address),
        Some(&Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT))
    );

    // Revert the transaction.
    let mut tx_logger = TransactionLog::empty();
    staking_contract
        .revert_outgoing_transaction(
            &tx,
            &block_state,
            receipt,
            data_store.write(&mut db_txn),
            &mut tx_logger,
        )
        .expect("Failed to revert transaction");

    assert_eq!(
        tx_logger.logs,
        vec![
            Log::Unstake {
                staker_address: staker_address.clone(),
                validator_address: Some(validator_address.clone()),
                value: Coin::from_u64_unchecked(50_000_000),
            },
            Log::Transfer {
                from: tx.sender.clone(),
                to: tx.recipient.clone(),
                amount: tx.value,
                data: None,
            },
            Log::PayFee {
                from: tx.sender.clone(),
                fee: tx.fee,
            },
        ]
    );

    let staker = staking_contract
        .get_staker(&data_store.read(&db_txn), &staker_address)
        .expect("Staker should exist");

    assert_eq!(staker.address, staker_address);
    assert_eq!(staker.balance, Coin::from_u64_unchecked(50_000_000));
    assert_eq!(staker.delegation, Some(validator_address.clone()));

    let validator = staking_contract
        .get_validator(&data_store.read(&db_txn), &validator_address)
        .expect("Validator should exist");

    assert_eq!(
        validator.total_stake,
        Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT + 50_000_000)
    );
    assert_eq!(validator.num_stakers, 1);

    assert_eq!(
        staking_contract.balance,
        Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT + 50_000_000)
    );

    assert_eq!(
        staking_contract.active_validators.get(&validator_address),
        Some(&Coin::from_u64_unchecked(
            Policy::VALIDATOR_DEPOSIT + 50_000_000
        ))
    );
}

#[test]
fn reward_inherents_not_allowed() {
    let env = VolatileDatabase::new(10).unwrap();
    let accounts = Accounts::new(env.clone());
    let data_store = accounts.data_store(&Policy::STAKING_CONTRACT_ADDRESS);
    let block_state = BlockState::new(2, 2);
    let mut db_txn = env.write_transaction();

    let mut staking_contract = make_sample_contract(data_store.write(&mut db_txn), true);

    let validator_address = validator_address();

    let inherent = Inherent::Reward {
        target: validator_address,
        value: Coin::ZERO,
    };

    assert_eq!(
        staking_contract.commit_inherent(
            &inherent,
            &block_state,
            data_store.write(&mut db_txn),
            &mut InherentLogger::empty()
        ),
        Err(AccountError::InvalidForTarget)
    );
}

#[test]
fn slash_inherents_work() {
    let env = VolatileDatabase::new(10).unwrap();
    let accounts = Accounts::new(env.clone());
    let data_store = accounts.data_store(&Policy::STAKING_CONTRACT_ADDRESS);
    let block_state = BlockState::new(2, 2);
    let mut db_txn = env.write_transaction();

    let mut staking_contract = make_sample_contract(data_store.write(&mut db_txn), true);

    let validator_address = validator_address();

    // Prepare some data.
    let slot = SlashedSlot {
        slot: 0,
        validator_address: validator_address.clone(),
        event_block: 2,
    };

    let inherent = Inherent::Slash { slot: slot.clone() };

    // Works in current epoch, current batch case.
    let mut logs = vec![];
    let mut inherent_logger = InherentLogger::new(&mut logs);
    let receipt = staking_contract
        .commit_inherent(
            &inherent,
            &block_state,
            data_store.write(&mut db_txn),
            &mut inherent_logger,
        )
        .expect("Failed to commit inherent");

    let expected_receipt = SlashReceipt {
        newly_deactivated: true,
        newly_disabled: true,
        newly_lost_rewards: true,
    };
    assert_eq!(receipt, Some(expected_receipt.into()));

    assert_eq!(
        logs,
        vec![
            Log::Slash {
                validator_address: slot.validator_address.clone(),
                event_block: slot.event_block,
                slot: slot.slot,
                newly_disabled: true,
                newly_deactivated: true,
            },
            Log::DeactivateValidator {
                validator_address: slot.validator_address.clone(),
            },
        ]
    );

    assert!(staking_contract
        .current_batch_lost_rewards
        .contains(slot.slot as usize));
    assert!(!staking_contract
        .previous_batch_lost_rewards
        .contains(slot.slot as usize));
    assert!(staking_contract
        .current_epoch_disabled_slots
        .get(&validator_address)
        .unwrap()
        .contains(&slot.slot));
    assert!(staking_contract
        .previous_epoch_disabled_slots
        .get(&validator_address)
        .is_none());

    revert_slash_inherent(
        &mut staking_contract,
        data_store.write(&mut db_txn),
        &inherent,
        &block_state,
        receipt,
        &validator_address,
        slot.slot,
    );

    // Works in current epoch, previous batch case.
    let block_state = BlockState::new(Policy::blocks_per_batch() + 1, 500);

    let mut logs = vec![];
    let mut inherent_logger = InherentLogger::new(&mut logs);
    let receipt = staking_contract
        .commit_inherent(
            &inherent,
            &block_state,
            data_store.write(&mut db_txn),
            &mut inherent_logger,
        )
        .expect("Failed to commit inherent");

    let expected_receipt = SlashReceipt {
        newly_deactivated: true,
        newly_disabled: true,
        newly_lost_rewards: true,
    };
    assert_eq!(receipt, Some(expected_receipt.into()));

    assert_eq!(
        logs,
        vec![
            Log::Slash {
                validator_address: slot.validator_address.clone(),
                event_block: 2,
                slot: slot.slot,
                newly_disabled: true,
                newly_deactivated: true,
            },
            Log::DeactivateValidator {
                validator_address: slot.validator_address.clone(),
            },
        ]
    );

    assert!(!staking_contract
        .current_batch_lost_rewards
        .contains(slot.slot as usize));
    assert!(staking_contract
        .previous_batch_lost_rewards
        .contains(slot.slot as usize));
    assert!(staking_contract
        .current_epoch_disabled_slots
        .get(&validator_address)
        .unwrap()
        .contains(&slot.slot));
    assert!(staking_contract
        .previous_epoch_disabled_slots
        .get(&validator_address)
        .is_none());

    revert_slash_inherent(
        &mut staking_contract,
        data_store.write(&mut db_txn),
        &inherent,
        &block_state,
        receipt,
        &validator_address,
        slot.slot,
    );

    // Works in previous epoch, previous batch case.
    let block_state = BlockState::new(Policy::blocks_per_epoch() + 1, 1000);

    let mut logs = vec![];
    let mut inherent_logger = InherentLogger::new(&mut logs);
    let receipt = staking_contract
        .commit_inherent(
            &inherent,
            &block_state,
            data_store.write(&mut db_txn),
            &mut inherent_logger,
        )
        .expect("Failed to commit inherent");

    let expected_receipt = SlashReceipt {
        newly_deactivated: true,
        newly_disabled: false,
        newly_lost_rewards: true,
    };
    assert_eq!(receipt, Some(expected_receipt.into()));

    assert_eq!(
        logs,
        vec![
            Log::Slash {
                validator_address: slot.validator_address.clone(),
                event_block: slot.event_block,
                slot: slot.slot,
                newly_disabled: false,
                newly_deactivated: true
            },
            Log::DeactivateValidator {
                validator_address: slot.validator_address,
            },
        ]
    );

    assert!(!staking_contract
        .current_batch_lost_rewards
        .contains(slot.slot as usize));
    assert!(staking_contract
        .previous_batch_lost_rewards
        .contains(slot.slot as usize));
    assert!(staking_contract
        .current_epoch_disabled_slots
        .get(&validator_address)
        .is_none());
    assert!(staking_contract
        .previous_epoch_disabled_slots
        .get(&validator_address)
        .is_none());

    revert_slash_inherent(
        &mut staking_contract,
        data_store.write(&mut db_txn),
        &inherent,
        &block_state,
        receipt,
        &validator_address,
        slot.slot,
    );
}

#[test]
fn finalize_batch_inherents_works() {
    let env = VolatileDatabase::new(10).unwrap();
    let accounts = Accounts::new(env.clone());
    let data_store = accounts.data_store(&Policy::STAKING_CONTRACT_ADDRESS);
    let block_state = BlockState::new(Policy::blocks_per_batch(), 500);
    let mut db_txn = env.write_transaction();

    let mut staking_contract = make_sample_contract(data_store.write(&mut db_txn), true);

    // Prepare the staking contract.
    staking_contract.current_batch_lost_rewards.insert(0);
    staking_contract.previous_batch_lost_rewards.insert(1);

    // Works in the valid case.
    let inherent = Inherent::FinalizeBatch;

    let mut logs = vec![];
    let mut inherent_logger = InherentLogger::new(&mut logs);
    let receipt = staking_contract
        .commit_inherent(
            &inherent,
            &block_state,
            data_store.write(&mut db_txn),
            &mut inherent_logger,
        )
        .expect("Failed to commit inherent");

    assert_eq!(receipt, None);
    assert!(logs.is_empty());

    assert!(staking_contract.current_batch_lost_rewards.is_empty());
    assert!(staking_contract.previous_batch_lost_rewards.contains(0));
    assert!(staking_contract.current_epoch_disabled_slots.is_empty());
    assert!(staking_contract.previous_epoch_disabled_slots.is_empty());

    // Cannot revert the inherent.
    assert_eq!(
        staking_contract.revert_inherent(
            &inherent,
            &block_state,
            None,
            data_store.write(&mut db_txn),
            &mut InherentLogger::empty()
        ),
        Err(AccountError::InvalidForTarget)
    );
}

#[test]
fn finalize_epoch_inherents_works() {
    let env = VolatileDatabase::new(10).unwrap();
    let accounts = Accounts::new(env.clone());
    let data_store = accounts.data_store(&Policy::STAKING_CONTRACT_ADDRESS);
    let block_state = BlockState::new(Policy::blocks_per_epoch(), 1000);
    let mut db_txn = env.write_transaction();

    let mut staking_contract = make_sample_contract(data_store.write(&mut db_txn), true);

    let validator_address = validator_address();

    // Pre polutate the previous epoch and batch related sets.
    // To test proper behaviour upon epoch finalization.
    staking_contract.previous_batch_lost_rewards.insert(10);
    staking_contract
        .previous_epoch_disabled_slots
        .insert(Address::END_ADDRESS, BTreeSet::new());

    // Slash the validator slot
    let inherent = Inherent::Slash {
        slot: SlashedSlot {
            slot: 1,
            validator_address: validator_address.clone(),
            event_block: Policy::blocks_per_epoch() - 1,
        },
    };
    let mut logs = vec![];
    let mut inherent_logger = InherentLogger::new(&mut logs);
    let receipt = staking_contract
        .commit_inherent(
            &inherent,
            &block_state,
            data_store.write(&mut db_txn),
            &mut inherent_logger,
        )
        .expect("Failed to commit inherent");
    assert_eq!(
        receipt,
        Some(
            SlashReceipt {
                newly_deactivated: true,
                newly_disabled: true,
                newly_lost_rewards: true
            }
            .into()
        )
    );
    assert_eq!(
        logs,
        vec![
            Log::Slash {
                validator_address: validator_address.clone(),
                event_block: Policy::blocks_per_epoch() - 1,
                slot: 1,
                newly_disabled: true,
                newly_deactivated: true
            },
            Log::DeactivateValidator {
                validator_address: validator_address.clone(),
            }
        ]
    );

    assert!(!staking_contract
        .active_validators
        .contains_key(&validator_address));

    // Finalize epoch to check that the relevant sets are set properly.
    let inherent = Inherent::FinalizeEpoch;

    let mut logs = vec![];
    let mut inherent_logger = InherentLogger::new(&mut logs);
    let receipt = staking_contract
        .commit_inherent(
            &inherent,
            &block_state,
            data_store.write(&mut db_txn),
            &mut inherent_logger,
        )
        .expect("Failed to commit inherent");

    assert_eq!(receipt, None);
    assert_eq!(logs, vec![]);

    assert!(staking_contract.current_batch_lost_rewards.is_empty());
    assert!(staking_contract.current_epoch_disabled_slots.is_empty());

    let mut bitset = BitSet::new();
    bitset.insert(1);
    assert_eq!(staking_contract.previous_batch_lost_rewards, bitset);
    let mut set_c = BTreeSet::new();
    set_c.insert(1);
    assert_eq!(
        staking_contract
            .previous_epoch_disabled_slots
            .get(&validator_address)
            .unwrap(),
        &set_c
    );

    let validator = staking_contract
        .get_validator(&data_store.read(&db_txn), &validator_address)
        .expect("Validator should exist");

    assert_eq!(validator.inactive_since, Some(Policy::blocks_per_epoch()));

    // Cannot revert the inherent.
    assert_eq!(
        staking_contract.revert_inherent(
            &inherent,
            &block_state,
            None,
            data_store.write(&mut db_txn),
            &mut InherentLogger::empty()
        ),
        Err(AccountError::InvalidForTarget)
    );
}

fn make_empty_contract() -> StakingContract {
    StakingContract::default()
}

fn make_sample_contract(mut data_store: DataStoreWrite, with_staker: bool) -> StakingContract {
    let staker_address = staker_address();

    let cold_address = validator_address();

    let signing_key =
        PublicKey::deserialize_from_vec(&hex::decode(VALIDATOR_SIGNING_KEY).unwrap()).unwrap();

    let voting_key =
        BlsPublicKey::deserialize_from_vec(&hex::decode(VALIDATOR_VOTING_KEY).unwrap()).unwrap();

    let mut contract = make_empty_contract();

    let mut store = StakingContractStoreWrite::new(&mut data_store);

    contract
        .create_validator(
            &mut store,
            &cold_address,
            signing_key,
            voting_key,
            cold_address.clone(),
            None,
            Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT),
            &mut TransactionLog::empty(),
        )
        .unwrap();

    if with_staker {
        contract
            .create_staker(
                &mut store,
                &staker_address,
                Coin::from_u64_unchecked(150_000_000),
                Some(cold_address),
                &mut TransactionLog::empty(),
            )
            .unwrap();
    }

    contract
}

fn make_incoming_transaction(data: IncomingStakingTransactionData, value: u64) -> Transaction {
    match data {
        IncomingStakingTransactionData::CreateValidator { .. }
        | IncomingStakingTransactionData::CreateStaker { .. }
        | IncomingStakingTransactionData::AddStake { .. } => Transaction::new_extended(
            staker_address(),
            AccountType::Basic,
            Policy::STAKING_CONTRACT_ADDRESS,
            AccountType::Staking,
            value.try_into().unwrap(),
            100.try_into().unwrap(),
            data.serialize_to_vec(),
            1,
            NetworkId::Dummy,
        ),
        _ => Transaction::new_signaling(
            staker_address(),
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
        Policy::STAKING_CONTRACT_ADDRESS,
        AccountType::Staking,
        staker_address(),
        AccountType::Basic,
        (Policy::VALIDATOR_DEPOSIT - 100).try_into().unwrap(),
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
        Policy::STAKING_CONTRACT_ADDRESS,
        AccountType::Staking,
        staker_address(),
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

    let proof = OutgoingStakingTransactionProof::RemoveStake { proof: sig };

    tx.proof = proof.serialize_to_vec();

    tx
}

fn revert_slash_inherent(
    staking_contract: &mut StakingContract,
    data_store: DataStoreWrite,
    inherent: &Inherent,
    block_state: &BlockState,
    receipt: Option<AccountReceipt>,
    validator_address: &Address,
    slot: u16,
) {
    let newly_disabled = staking_contract
        .current_epoch_disabled_slots
        .contains_key(validator_address);
    let newly_deactivated = !staking_contract
        .active_validators
        .contains_key(validator_address);
    let mut logs = vec![];
    let mut inherent_logger = InherentLogger::new(&mut logs);
    staking_contract
        .revert_inherent(
            inherent,
            block_state,
            receipt,
            data_store,
            &mut inherent_logger,
        )
        .expect("Failed to revert inherent");

    let mut event_block = block_state.number;
    if let Inherent::Slash { ref slot } = inherent {
        event_block = slot.event_block;
    }

    assert_eq!(
        logs,
        vec![
            Log::DeactivateValidator {
                validator_address: validator_address.clone(),
            },
            Log::Slash {
                validator_address: validator_address.clone(),
                event_block,
                slot,
                newly_disabled,
                newly_deactivated,
            },
        ]
    );

    assert!(!staking_contract
        .current_batch_lost_rewards
        .contains(slot as usize));
    assert!(!staking_contract
        .previous_batch_lost_rewards
        .contains(slot as usize));
    assert!(staking_contract
        .current_epoch_disabled_slots
        .get(validator_address)
        .is_none());
    assert!(staking_contract
        .previous_epoch_disabled_slots
        .get(validator_address)
        .is_none());
}

fn bls_key_pair(sk: &str) -> BlsKeyPair {
    BlsKeyPair::from(BlsSecretKey::deserialize_from_vec(&hex::decode(sk).unwrap()).unwrap())
}

fn bls_public_key(pk: &str) -> BlsPublicKey {
    BlsPublicKey::deserialize_from_vec(&hex::decode(pk).unwrap()).unwrap()
}

fn ed25519_key_pair(sk: &str) -> KeyPair {
    KeyPair::from(PrivateKey::deserialize_from_vec(&hex::decode(sk).unwrap()).unwrap())
}

fn ed25519_public_key(pk: &str) -> PublicKey {
    PublicKey::deserialize_from_vec(&hex::decode(pk).unwrap()).unwrap()
}

fn validator_address() -> Address {
    Address::from_hex(VALIDATOR_ADDRESS).unwrap()
}

fn staker_address() -> Address {
    Address::from_hex(STAKER_ADDRESS).unwrap()
}
