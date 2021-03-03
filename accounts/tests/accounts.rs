use std::convert::TryFrom;

use beserial::Serialize;
use nimiq_account::{
    Account, AccountTransactionInteraction, AccountType, BasicAccount, Inherent, InherentType,
    PrunedAccount,
};
use nimiq_account::{Receipt, Receipts};
use nimiq_accounts::Accounts;
use nimiq_database::volatile::VolatileEnvironment;
use nimiq_database::ReadTransaction;
use nimiq_database::WriteTransaction;
use nimiq_keys::{Address, KeyPair, SecureGenerate};
use nimiq_primitives::coin::Coin;
use nimiq_primitives::networks::NetworkId;
use nimiq_transaction::{SignatureProof, Transaction};

#[test]
fn it_can_commit_and_revert_a_block_body() {
    let env = VolatileEnvironment::new(10).unwrap();

    let accounts = Accounts::new(env.clone());

    let address_validator = Address::from([1u8; Address::SIZE]);

    let address_recipient = Address::from([2u8; Address::SIZE]);

    let reward = Inherent {
        ty: InherentType::Reward,
        target: address_validator.clone(),
        value: Coin::from_u64_unchecked(10000),
        data: vec![],
    };

    let receipts = Receipts::default();

    assert_eq!(accounts.get(&address_validator, None).balance(), Coin::ZERO);

    let mut txn = WriteTransaction::new(&env);

    assert!(accounts
        .commit(&mut txn, &[], &[reward.clone()], 1, 1)
        .is_ok());

    txn.commit();

    assert_eq!(
        accounts.get(&address_validator, None).balance(),
        Coin::from_u64_unchecked(10000)
    );

    let hash1 = accounts.hash(None);

    let tx = Transaction::new_basic(
        address_validator.clone(),
        address_recipient.clone(),
        Coin::from_u64_unchecked(10),
        Coin::ZERO,
        1,
        NetworkId::Main,
    );

    let transactions = vec![tx];

    assert_eq!(accounts.get(&address_recipient, None).balance(), Coin::ZERO);

    let mut txn = WriteTransaction::new(&env);

    assert!(accounts
        .commit(&mut txn, &transactions, &[reward.clone()], 2, 2)
        .is_ok());

    txn.commit();

    assert_eq!(
        accounts.get(&address_recipient, None).balance(),
        Coin::from_u64_unchecked(10)
    );

    assert_eq!(
        accounts.get(&address_validator, None).balance(),
        Coin::from_u64_unchecked(10000 + 10000 - 10)
    );

    assert_ne!(hash1, accounts.hash(None));

    let mut txn = WriteTransaction::new(&env);

    assert!(accounts
        .revert(&mut txn, &transactions, &[reward], 2, 2, &receipts)
        .is_ok());

    txn.commit();

    assert_eq!(accounts.get(&address_recipient, None).balance(), Coin::ZERO);

    assert_eq!(
        accounts.get(&address_validator, None).balance(),
        Coin::from_u64_unchecked(10000)
    );

    assert_eq!(hash1, accounts.hash(None));
}

#[test]
fn it_correctly_rewards_validators() {
    let env = VolatileEnvironment::new(10).unwrap();

    let accounts = Accounts::new(env.clone());

    let address_validator_1 = Address::from([1u8; Address::SIZE]);

    let address_validator_2 = Address::from([2u8; Address::SIZE]);

    let address_recipient_1 = Address::from([3u8; Address::SIZE]);

    let address_recipient_2 = Address::from([4u8; Address::SIZE]);

    // Validator 1 mines first block.
    assert_eq!(
        accounts.get(&address_validator_1, None).balance(),
        Coin::ZERO
    );

    let reward = Inherent {
        ty: InherentType::Reward,
        target: address_validator_1.clone(),
        value: Coin::from_u64_unchecked(10000),
        data: vec![],
    };

    let mut txn = WriteTransaction::new(&env);

    assert!(accounts.commit(&mut txn, &[], &[reward], 1, 1).is_ok());

    txn.commit();

    // Create transactions to Recipient 1 and Recipient 2.
    assert_eq!(
        accounts.get(&address_validator_1, None).balance(),
        Coin::from_u64_unchecked(10000)
    );

    let value1 = Coin::from_u64_unchecked(5);

    let fee1 = Coin::from_u64_unchecked(3);

    let value2 = Coin::from_u64_unchecked(7);

    let fee2 = Coin::from_u64_unchecked(11);

    let tx1 = Transaction::new_basic(
        address_validator_1.clone(),
        address_recipient_1.clone(),
        value1,
        fee1,
        2,
        NetworkId::Main,
    );

    let tx2 = Transaction::new_basic(
        address_validator_1.clone(),
        address_recipient_2.clone(),
        value2,
        fee2,
        2,
        NetworkId::Main,
    );

    // Validator 2 mines second block.
    assert_eq!(
        accounts.get(&address_validator_2, None).balance(),
        Coin::ZERO
    );

    let reward = Inherent {
        ty: InherentType::Reward,
        target: address_validator_2.clone(),
        value: Coin::from_u64_unchecked(10000) + fee1 + fee2,
        data: vec![],
    };

    let mut txn = WriteTransaction::new(&env);

    assert!(accounts
        .commit(&mut txn, &vec![tx1, tx2], &[reward], 2, 2)
        .is_ok());

    txn.commit();

    assert_eq!(
        accounts.get(&address_validator_1, None).balance(),
        Coin::from_u64_unchecked(10000) - value1 - fee1 - value2 - fee2
    );

    assert_eq!(
        accounts.get(&address_validator_2, None).balance(),
        Coin::from_u64_unchecked(10000) + fee1 + fee2
    );

    assert_eq!(accounts.get(&address_recipient_1, None).balance(), value1);

    assert_eq!(accounts.get(&address_recipient_2, None).balance(), value2);
}

#[test]
fn it_checks_for_sufficient_funds() {
    let env = VolatileEnvironment::new(10).unwrap();

    let accounts = Accounts::new(env.clone());

    let address_sender = Address::from([1u8; Address::SIZE]);

    let address_recipient = Address::from([2u8; Address::SIZE]);

    let mut tx = Transaction::new_basic(
        address_sender.clone(),
        address_recipient.clone(),
        Coin::try_from(10).unwrap(),
        Coin::ZERO,
        1,
        NetworkId::Main,
    );

    let reward = Inherent {
        ty: InherentType::Reward,
        target: address_sender.clone(),
        value: Coin::from_u64_unchecked(10000),
        data: vec![],
    };

    let hash1 = accounts.hash(None);

    assert_eq!(accounts.get(&address_sender, None).balance(), Coin::ZERO);

    assert_eq!(accounts.get(&address_recipient, None).balance(), Coin::ZERO);

    // Fails as address_sender does not have any funds.
    // Note: When the commit errors, we want to bracket the txn creation and the commit attempt.
    // Otherwise when we try to commit again, the test will get stuck.
    {
        let mut txn = WriteTransaction::new(&env);

        assert!(accounts
            .commit(&mut txn, &[tx.clone()], &[reward.clone()], 1, 1)
            .is_err());
    }

    assert_eq!(accounts.get(&address_sender, None).balance(), Coin::ZERO);

    assert_eq!(accounts.get(&address_recipient, None).balance(), Coin::ZERO);

    assert_eq!(hash1, accounts.hash(None));

    // Give address_sender one block reward.

    let mut txn = WriteTransaction::new(&env);

    assert!(accounts
        .commit(&mut txn, &[], &[reward.clone()], 1, 1)
        .is_ok());

    txn.commit();

    assert_eq!(
        accounts.get(&address_sender, None).balance(),
        Coin::from_u64_unchecked(10000)
    );

    assert_eq!(accounts.get(&address_recipient, None).balance(), Coin::ZERO);

    let hash2 = accounts.hash(None);

    assert_ne!(hash1, hash2);

    // Single transaction exceeding funds.
    tx.value = Coin::from_u64_unchecked(1000000);

    {
        let mut txn = WriteTransaction::new(&env);

        assert!(accounts
            .commit(&mut txn, &[tx.clone()], &[reward.clone()], 2, 2)
            .is_err());
    }

    assert_eq!(
        accounts.get(&address_sender, None).balance(),
        Coin::from_u64_unchecked(10000)
    );

    assert_eq!(accounts.get(&address_recipient, None).balance(), Coin::ZERO);

    assert_eq!(hash2, accounts.hash(None));

    // Multiple transactions exceeding funds.
    tx.value = Coin::from_u64_unchecked(5010);

    let mut tx2 = tx.clone();

    tx2.value += Coin::from_u64_unchecked(10);

    {
        let mut txn = WriteTransaction::new(&env);

        assert!(accounts
            .commit(&mut txn, &vec![tx, tx2], &[reward], 2, 2)
            .is_err());
    }

    assert_eq!(
        accounts.get(&address_sender, None).balance(),
        Coin::from_u64_unchecked(10000)
    );

    assert_eq!(accounts.get(&address_recipient, None).balance(), Coin::ZERO);

    assert_eq!(hash2, accounts.hash(None));
}

#[test]
fn it_correctly_prunes_account() {
    let env = VolatileEnvironment::new(10).unwrap();

    let accounts = Accounts::new(env.clone());

    let key_pair = KeyPair::generate_default_csprng();

    let address = Address::from(&key_pair.public);

    let reward = Inherent {
        ty: InherentType::Reward,
        target: address.clone(),
        value: Coin::from_u64_unchecked(10000),
        data: vec![],
    };

    // Give a block reward
    let mut txn = WriteTransaction::new(&env);

    assert!(accounts
        .commit(&mut txn, &[], &[reward.clone()], 1, 1)
        .is_ok());

    txn.commit();

    // Create vesting contract
    let initial_hash = accounts.hash(None);

    let mut data: Vec<u8> = Vec::with_capacity(Address::SIZE + 8);

    address.serialize(&mut data).unwrap();

    1u64.serialize(&mut data).unwrap();

    let mut tx_create = Transaction::new_contract_creation(
        data,
        address.clone(),
        AccountType::Basic,
        AccountType::Vesting,
        Coin::from_u64_unchecked(100),
        Coin::from_u64_unchecked(0),
        1,
        NetworkId::Dummy,
    );

    tx_create.proof = SignatureProof::from(
        key_pair.public,
        key_pair.sign(&tx_create.serialize_content()),
    )
    .serialize_to_vec();

    let contract_address = tx_create.contract_creation_address();

    let mut txn = WriteTransaction::new(&env);

    assert!(accounts
        .commit(&mut txn, &[tx_create.clone()], &[reward.clone()], 2, 2)
        .is_ok());

    txn.commit();

    // Now prune it
    let mut tx_prune = Transaction::new_basic(
        contract_address.clone(),
        address.clone(),
        Coin::try_from(100).unwrap(),
        Coin::try_from(0).unwrap(),
        2,
        NetworkId::Dummy,
    );

    tx_prune.sender_type = AccountType::Vesting;

    tx_prune.proof = SignatureProof::from(
        key_pair.public,
        key_pair.sign(&tx_prune.serialize_content()),
    )
    .serialize_to_vec();

    let mut pruned_account = accounts.get(&contract_address, None);

    pruned_account
        .commit_outgoing_transaction(&tx_prune, 2, 2)
        .unwrap();

    let receipts = Receipts {
        receipts: vec![Receipt::PrunedAccount(PrunedAccount {
            address: contract_address.clone(),
            account: pruned_account,
        })],
    };

    let mut txn = WriteTransaction::new(&env);

    assert_eq!(
        accounts.commit(&mut txn, &[tx_prune.clone()], &[reward.clone()], 3, 3),
        Ok(receipts.clone())
    );

    txn.commit();

    // Check that the account was pruned correctly
    let account_after_prune = accounts.get(&contract_address, None);

    assert_eq!(account_after_prune.account_type(), AccountType::Basic);

    assert_eq!(account_after_prune.balance(), Coin::try_from(0).unwrap());

    // Now revert pruning
    let mut txn = WriteTransaction::new(&env);

    assert!(accounts
        .revert(&mut txn, &[tx_prune], &[reward.clone()], 3, 3, &receipts)
        .is_ok());

    txn.commit();

    // Check that the account was recovered correctly
    if let Account::Vesting(vesting_contract) = accounts.get(&contract_address, None) {
        assert_eq!(vesting_contract.balance, Coin::try_from(100).unwrap());
        assert_eq!(vesting_contract.owner, address);
    }

    // Now revert account
    let mut txn = WriteTransaction::new(&env);

    assert!(accounts
        .revert(
            &mut txn,
            &[tx_create],
            &[reward],
            2,
            2,
            &Receipts::default()
        )
        .is_ok());

    txn.commit();

    // Check that the account is really gone
    let account_after_prune = accounts.get(&contract_address, None);

    assert_eq!(account_after_prune.account_type(), AccountType::Basic);

    assert_eq!(account_after_prune.balance(), Coin::try_from(0).unwrap());

    assert_eq!(accounts.hash(None), initial_hash);
}

#[test]
fn can_generate_accounts_proof() {
    let env = VolatileEnvironment::new(10).unwrap();

    let accounts = Accounts::new(env.clone());

    let address_validator_1 = Address::from([1u8; Address::SIZE]);

    let address_validator_2 = Address::from([2u8; Address::SIZE]);

    let address_recipient_1 = Address::from([3u8; Address::SIZE]);

    let address_recipient_2 = Address::from([4u8; Address::SIZE]);

    let reward = Inherent {
        ty: InherentType::Reward,
        target: address_validator_1.clone(),
        value: Coin::from_u64_unchecked(10000),
        data: vec![],
    };

    let mut txn = WriteTransaction::new(&env);

    assert!(accounts.commit(&mut txn, &[], &[reward], 1, 1).is_ok());

    txn.commit();

    let value1 = Coin::from_u64_unchecked(5);

    let fee1 = Coin::from_u64_unchecked(3);

    let value2 = Coin::from_u64_unchecked(7);

    let fee2 = Coin::from_u64_unchecked(11);

    let tx1 = Transaction::new_basic(
        address_validator_1.clone(),
        address_recipient_1.clone(),
        value1,
        fee1,
        2,
        NetworkId::Main,
    );

    let tx2 = Transaction::new_basic(
        address_validator_1.clone(),
        address_recipient_2.clone(),
        value2,
        fee2,
        2,
        NetworkId::Main,
    );

    let reward = Inherent {
        ty: InherentType::Reward,
        target: address_validator_2.clone(),
        value: Coin::from_u64_unchecked(10000) + fee1 + fee2,
        data: vec![],
    };

    let mut txn = WriteTransaction::new(&env);

    assert!(accounts
        .commit(&mut txn, &vec![tx1, tx2], &[reward], 2, 2)
        .is_ok());

    txn.commit();

    let mut read_accs_txn = ReadTransaction::new(&env);

    let mut proof1 =
        accounts.get_accounts_proof(&mut read_accs_txn, &[address_validator_1.clone()]);

    assert!(proof1.verify());

    assert_eq!(
        Account::Basic(BasicAccount {
            balance: Coin::from_u64_unchecked(10000) - value1 - fee1 - value2 - fee2
        }),
        proof1.get_account(&address_validator_1).unwrap()
    );

    assert_eq!(None, proof1.get_account(&address_validator_2));

    assert_eq!(None, proof1.get_account(&address_recipient_1));

    assert_eq!(None, proof1.get_account(&address_recipient_2));

    let mut proof2 = accounts.get_accounts_proof(
        &mut read_accs_txn,
        &[address_recipient_2.clone(), address_validator_2.clone()],
    );

    assert!(proof2.verify());

    assert_eq!(
        Account::Basic(BasicAccount {
            balance: Coin::from_u64_unchecked(10000) + fee1 + fee2
        }),
        proof2.get_account(&address_validator_2).unwrap()
    );

    assert_eq!(None, proof2.get_account(&address_validator_1));

    assert_eq!(None, proof2.get_account(&address_recipient_1));

    assert_eq!(
        Account::Basic(BasicAccount { balance: value2 }),
        proof2.get_account(&address_recipient_2).unwrap()
    );
}
