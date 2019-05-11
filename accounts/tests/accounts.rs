use beserial::Serialize;
use nimiq_account::{Account, AccountTransactionInteraction, AccountType, BasicAccount, PrunedAccount};
use nimiq_account::Receipt;
use nimiq_accounts::Accounts;
use nimiq_block::BlockBody;
use nimiq_database::ReadTransaction;
use nimiq_database::volatile::VolatileEnvironment;
use nimiq_database::WriteTransaction;
use nimiq_keys::{Address, KeyPair};
use nimiq_primitives::coin::Coin;
use nimiq_primitives::networks::NetworkId;
use nimiq_primitives::policy;
use nimiq_transaction::{SignatureProof, Transaction};

#[test]
fn it_can_commit_and_revert_a_block_body() {
    let env = VolatileEnvironment::new(10).unwrap();
    let accounts = Accounts::new(&env);
    let address_miner = Address::from([1u8; Address::SIZE]);
    let address_recipient = Address::from([2u8; Address::SIZE]);

    let mut body = BlockBody {
        miner: address_miner.clone(),
        extra_data: Vec::new(),
        transactions: Vec::new(),
        receipts: Vec::new()
    };

    assert_eq!(accounts.get(&address_miner, None).balance(), Coin::ZERO);
    {
        let mut txn = WriteTransaction::new(&env);
        assert!(accounts.commit(&mut txn, &body.transactions, &vec![body.get_reward_inherent(1)], 1).is_ok());
        txn.commit();
    }

    assert_eq!(accounts.get(&address_miner, None).balance(), policy::block_reward_at(1));
    let hash1 = accounts.hash(None);

    let tx = Transaction::new_basic(
        address_miner.clone(),
        address_recipient.clone(),
        Coin::from_u64(10).unwrap(),
        Coin::ZERO,
        1,
        NetworkId::Main
    );
    body.transactions = vec![tx];

    assert_eq!(accounts.get(&address_recipient, None).balance(), Coin::ZERO);
    {
        let mut txn = WriteTransaction::new(&env);
        assert!(accounts.commit(&mut txn, &body.transactions, &vec![body.get_reward_inherent(2)], 2).is_ok());
        txn.commit();
    }

    assert_eq!(accounts.get(&address_recipient, None).balance(), Coin::from_u64(10).unwrap());
    assert_eq!(accounts.get(&address_miner, None).balance(),
               policy::block_reward_at(1) + policy::block_reward_at(2) - Coin::from_u64(10).unwrap());
    assert_ne!(hash1, accounts.hash(None));

    {
        let mut txn = WriteTransaction::new(&env);
        assert!(accounts.revert(&mut txn, &body.transactions, &vec![body.get_reward_inherent(2)], 2, &body.receipts).is_ok());
        txn.commit();
    }

    assert_eq!(accounts.get(&address_recipient, None).balance(), Coin::ZERO);
    assert_eq!(accounts.get(&address_miner, None).balance(), policy::block_reward_at(1));
    assert_eq!(hash1, accounts.hash(None));
}

#[test]
fn it_can_deal_with_multiple_transactions_per_sender() {

}

#[test]
fn it_correctly_rewards_miners() {
    let env = VolatileEnvironment::new(10).unwrap();
    let accounts = Accounts::new(&env);
    let address_miner1 = Address::from([1u8; Address::SIZE]);
    let address_miner2 = Address::from([2u8; Address::SIZE]);
    let address_recipient1 = Address::from([3u8; Address::SIZE]);
    let address_recipient2 = Address::from([4u8; Address::SIZE]);

    let mut body = BlockBody {
        miner: address_miner1.clone(),
        extra_data: Vec::new(),
        transactions: Vec::new(),
        receipts: Vec::new()
    };

    // address_miner1 mines first block.
    assert_eq!(accounts.get(&address_miner1, None).balance(), Coin::ZERO);
    {
        let mut txn = WriteTransaction::new(&env);
        assert!(accounts.commit(&mut txn, &body.transactions, &vec![body.get_reward_inherent(1)], 1).is_ok());
        txn.commit();
    }

    assert_eq!(accounts.get(&address_miner1, None).balance(), policy::block_reward_at(1));

    let value1 = Coin::from_u64(5).unwrap();
    let fee1 = Coin::from_u64(3).unwrap();
    let value2 = Coin::from_u64(7).unwrap();
    let fee2 = Coin::from_u64(11).unwrap();
    let tx1 = Transaction::new_basic(
        address_miner1.clone(),
        address_recipient1.clone(),
        value1,
        fee1,
        2,
        NetworkId::Main
    );
    let tx2 = Transaction::new_basic(
        address_miner1.clone(),
        address_recipient2.clone(),
        value2,
        fee2,
        2,
        NetworkId::Main
    );

    // address_miner2 mines second block.
    body.miner = address_miner2.clone();
    body.transactions = vec![tx1, tx2];

    assert_eq!(accounts.get(&address_miner2, None).balance(), Coin::ZERO);
    {
        let mut txn = WriteTransaction::new(&env);
        assert!(accounts.commit(&mut txn, &body.transactions, &vec![body.get_reward_inherent(2)], 2).is_ok());
        txn.commit();
    }

    assert_eq!(accounts.get(&address_miner1, None).balance(), policy::block_reward_at(1) - value1 - fee1 - value2 - fee2);
    assert_eq!(accounts.get(&address_miner2, None).balance(), policy::block_reward_at(2) + fee1 + fee2);
    assert_eq!(accounts.get(&address_recipient1, None).balance(), value1);
    assert_eq!(accounts.get(&address_recipient2, None).balance(), value2);
}

#[test]
fn it_checks_for_sufficient_funds() {
    let env = VolatileEnvironment::new(10).unwrap();
    let accounts = Accounts::new(&env);
    let address_sender = Address::from([1u8; Address::SIZE]);
    let address_recipient = Address::from([2u8; Address::SIZE]);

    let mut tx = Transaction::new_basic(
        address_sender.clone(),
        address_recipient.clone(),
        Coin::from_u64(10).unwrap(),
        Coin::ZERO,
        1,
        NetworkId::Main
    );

    let mut body = BlockBody {
        miner: address_sender.clone(),
        extra_data: Vec::new(),
        transactions: vec![tx.clone()],
        receipts: Vec::new()
    };

    let hash1 = accounts.hash(None);
    assert_eq!(accounts.get(&address_sender, None).balance(), Coin::ZERO);
    assert_eq!(accounts.get(&address_recipient, None).balance(), Coin::ZERO);

    // Fails as address_sender does not have any funds.
    {
        let mut txn = WriteTransaction::new(&env);
        assert!(accounts.commit(&mut txn, &body.transactions, &vec![body.get_reward_inherent(1)], 1).is_err());
    }

    assert_eq!(accounts.get(&address_sender, None).balance(), Coin::ZERO);
    assert_eq!(accounts.get(&address_recipient, None).balance(), Coin::ZERO);
    assert_eq!(hash1, accounts.hash(None));

    // Give address_sender one block reward.
    body.transactions = Vec::new();

    {
        let mut txn = WriteTransaction::new(&env);
        assert!(accounts.commit(&mut txn, &body.transactions, &vec![body.get_reward_inherent(1)], 1).is_ok());
        txn.commit();
    }

    assert_eq!(accounts.get(&address_sender, None).balance(), policy::block_reward_at(1));
    assert_eq!(accounts.get(&address_recipient, None).balance(), Coin::ZERO);
    let hash2 = accounts.hash(None);
    assert_ne!(hash1, hash2);

    // Single transaction exceeding funds.
    tx.value = policy::block_reward_at(1) + Coin::from_u64(10).unwrap();
    body.transactions = vec![tx.clone()];

    {
        let mut txn = WriteTransaction::new(&env);
        assert!(accounts.commit(&mut txn, &body.transactions, &vec![body.get_reward_inherent(2)], 2).is_err());
    }

    assert_eq!(accounts.get(&address_sender, None).balance(), policy::block_reward_at(1));
    assert_eq!(accounts.get(&address_recipient, None).balance(), Coin::ZERO);
    assert_eq!(hash2, accounts.hash(None));

    // Multiple transactions exceeding funds.
    tx.value = Coin::from_u64(u64::from(policy::block_reward_at(1)) / 2 + 10).unwrap();
    let mut tx2 = tx.clone();
    tx2.value = tx2.value + Coin::from_u64(10).unwrap();
    body.transactions = vec![tx, tx2];

    {
        let mut txn = WriteTransaction::new(&env);
        assert!(accounts.commit(&mut txn, &body.transactions, &vec![body.get_reward_inherent(2)], 2).is_err());
    }

    assert_eq!(accounts.get(&address_sender, None).balance(), policy::block_reward_at(1));
    assert_eq!(accounts.get(&address_recipient, None).balance(), Coin::ZERO);
    assert_eq!(hash2, accounts.hash(None));
}

#[test]
fn it_prevents_spending_of_funds_received_in_the_same_block() {

}

#[test]
fn it_correctly_prunes_account() {
    let env = VolatileEnvironment::new(10).unwrap();
    let accounts = Accounts::new(&env);
    let key_pair = KeyPair::generate();
    let address = Address::from(&key_pair.public);
    let mut body = BlockBody {
        miner: address.clone(),
        extra_data: Vec::new(),
        transactions: Vec::new(),
        receipts: Vec::new()
    };

    // Give a block reward
    {
        let mut txn = WriteTransaction::new(&env);
        assert!(accounts.commit(&mut txn, &body.transactions, &vec![body.get_reward_inherent(1)], 1).is_ok());
        txn.commit();
    }

    // Create vesting contract
    let initial_hash = accounts.hash(None);
    let mut data: Vec<u8> = Vec::with_capacity(Address::SIZE + 4);
    address.serialize(&mut data).unwrap();
    1u32.serialize(&mut data).unwrap();
    let mut tx_create = Transaction::new_contract_creation(data, address.clone(), AccountType::Basic, AccountType::Vesting, Coin::from_u64(100).unwrap(), Coin::from_u64(0).unwrap(), 1, NetworkId::Dummy);
    tx_create.proof = SignatureProof::from(key_pair.public, key_pair.sign(&tx_create.serialize_content())).serialize_to_vec();
    let contract_address = tx_create.contract_creation_address();
    body.transactions = vec![tx_create.clone()];
    {
        let mut txn = WriteTransaction::new(&env);
        assert!(accounts.commit(&mut txn, &body.transactions, &vec![body.get_reward_inherent(2)], 2).is_ok());
        txn.commit();
    }

    // Now prune it
    let mut tx_prune = Transaction::new_basic(contract_address.clone(), address.clone(), Coin::from_u64(100).unwrap(), Coin::from_u64(0).unwrap(), 2, NetworkId::Dummy);
    tx_prune.sender_type = AccountType::Vesting;
    tx_prune.proof = SignatureProof::from(key_pair.public, key_pair.sign(&tx_prune.serialize_content())).serialize_to_vec();
    body.transactions = vec![tx_prune.clone()];
    let mut pruned_account = accounts.get(&contract_address, None);
    pruned_account.commit_outgoing_transaction(&tx_prune, 2).unwrap();
    body.receipts = vec![Receipt::PrunedAccount(PrunedAccount {
        address: contract_address.clone(),
        account: pruned_account
    })];
    {
        let mut txn = WriteTransaction::new(&env);
        assert_eq!(accounts.commit(&mut txn, &body.transactions, &vec![body.get_reward_inherent(3)], 3), Ok(body.receipts.clone()));
        txn.commit();
    }

    // Check that the account was pruned correctly
    let account_after_prune = accounts.get(&contract_address, None);
    assert_eq!(account_after_prune.account_type(), AccountType::Basic);
    assert_eq!(account_after_prune.balance(), Coin::from_u64(0).unwrap());

    // Now revert pruning
    {
        let mut txn = WriteTransaction::new(&env);
        assert!(accounts.revert(&mut txn, &body.transactions, &vec![body.get_reward_inherent(3)], 3, &body.receipts).is_ok());
        txn.commit();
    }

    // Check that the account was recovered correctly
    if let Account::Vesting(vesting_contract) = accounts.get(&contract_address, None) {
        assert_eq!(vesting_contract.balance, Coin::from_u64(100).unwrap());
        assert_eq!(vesting_contract.owner, address);
    }

    // Now revert account
    body.receipts = Vec::new();
    body.transactions = vec![tx_create.clone()];
    {
        let mut txn = WriteTransaction::new(&env);
        assert!(accounts.revert(&mut txn, &body.transactions, &vec![body.get_reward_inherent(2)], 2, &body.receipts).is_ok());
        txn.commit();
    }

    // Check that the account is really gone
    let account_after_prune = accounts.get(&contract_address, None);
    assert_eq!(account_after_prune.account_type(), AccountType::Basic);
    assert_eq!(account_after_prune.balance(), Coin::from_u64(0).unwrap());
    assert_eq!(accounts.hash(None), initial_hash);
}

#[test]
fn can_generate_accounts_proof() {
    let env = VolatileEnvironment::new(10).unwrap();
    let accounts = Accounts::new(&env);
    let address_miner1 = Address::from([1u8; Address::SIZE]);
    let address_miner2 = Address::from([2u8; Address::SIZE]);
    let address_recipient1 = Address::from([3u8; Address::SIZE]);
    let address_recipient2 = Address::from([4u8; Address::SIZE]);

    let mut body = BlockBody { miner: address_miner1.clone(), extra_data: Vec::new(), transactions: Vec::new(), receipts: Vec::new() };

    {
        let mut txn = WriteTransaction::new(&env);
        assert!(accounts.commit(&mut txn, &body.transactions, &vec![body.get_reward_inherent(1)], 1).is_ok());
        txn.commit();
    }
    let value1 = Coin::from_u64(5).unwrap();
    let fee1 = Coin::from_u64(3).unwrap();
    let value2 = Coin::from_u64(7).unwrap();
    let fee2 = Coin::from_u64(11).unwrap();
    let tx1 = Transaction::new_basic(address_miner1.clone(), address_recipient1.clone(), value1, fee1, 2, NetworkId::Main);
    let tx2 = Transaction::new_basic(address_miner1.clone(), address_recipient2.clone(), value2, fee2, 2, NetworkId::Main);

    body.miner = address_miner2.clone();
    body.transactions = vec![tx1, tx2];

    {
        let mut txn = WriteTransaction::new(&env);
        assert!(accounts.commit(&mut txn, &body.transactions, &vec![body.get_reward_inherent(2)], 2).is_ok());
        txn.commit();
    }

    let mut read_accs_txn = ReadTransaction::new(&env);
    let mut proof1 = accounts.get_accounts_proof(&mut read_accs_txn, &vec![ address_miner1.clone() ]);
    assert!(proof1.verify());
    assert_eq!(Account::Basic(BasicAccount { balance: policy::block_reward_at(1) - value1 - fee1 - value2 - fee2 }), proof1.get_account(&address_miner1).unwrap());
    assert_eq!(None, proof1.get_account(&address_miner2));
    assert_eq!(None, proof1.get_account(&address_recipient1));
    assert_eq!(None, proof1.get_account(&address_recipient2));

    let mut proof2 = accounts.get_accounts_proof(&mut read_accs_txn, &vec![ address_recipient2.clone(), address_miner2.clone() ]);
    assert!(proof2.verify());
    assert_eq!(Account::Basic(BasicAccount { balance: policy::block_reward_at(2) + fee1 + fee2 }), proof2.get_account(&address_miner2).unwrap());
    assert_eq!(None, proof2.get_account(&address_miner1));
    assert_eq!(None, proof2.get_account(&address_recipient1));
    assert_eq!(Account::Basic(BasicAccount { balance: value2 }), proof2.get_account(&address_recipient2).unwrap());
}
