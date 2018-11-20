use beserial::Serialize;
use nimiq::consensus::base::account::{AccountError, Accounts, Account, AccountType, PrunedAccount};
use nimiq::consensus::base::block::{Block, BlockBody, BlockHeader, BlockInterlink, TargetCompact};
use nimiq::consensus::base::primitive::{Address, Coin, crypto::KeyPair, hash::{Blake2bHash, Hash}};
use nimiq::consensus::base::transaction::{SignatureProof, Transaction};
use nimiq::consensus::networks::NetworkId;
use nimiq::consensus::policy;
use nimiq::utils::db::volatile::VolatileEnvironment;
use nimiq::utils::db::WriteTransaction;

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
        pruned_accounts: Vec::new()
    };

    assert_eq!(accounts.get(&address_miner, None).balance(), Coin::ZERO);
    {
        let mut txn = WriteTransaction::new(&env);
        assert!(accounts.commit_block_body(&mut txn, &body, 1).is_ok());
        txn.commit();
    }

    assert_eq!(accounts.get(&address_miner, None).balance(), policy::block_reward_at(1));
    let hash1 = accounts.hash(None);

    let tx = Transaction::new_basic(
        address_miner.clone(),
        address_recipient.clone(),
        10.into(),
        Coin::ZERO,
        1,
        NetworkId::Main
    );
    body.transactions = vec![tx];

    assert_eq!(accounts.get(&address_recipient, None).balance(), Coin::ZERO);
    {
        let mut txn = WriteTransaction::new(&env);
        assert!(accounts.commit_block_body(&mut txn, &body, 2).is_ok());
        txn.commit();
    }

    assert_eq!(accounts.get(&address_recipient, None).balance(), Coin::from(10));
    assert_eq!(accounts.get(&address_miner, None).balance(),
               policy::block_reward_at(1) + policy::block_reward_at(2) - Coin::from(10));
    assert_ne!(hash1, accounts.hash(None));

    {
        let mut txn = WriteTransaction::new(&env);
        assert!(accounts.revert_block_body(&mut txn, &body, 2).is_ok());
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
        pruned_accounts: Vec::new()
    };

    // address_miner1 mines first block.
    assert_eq!(accounts.get(&address_miner1, None).balance(), Coin::ZERO);
    {
        let mut txn = WriteTransaction::new(&env);
        assert!(accounts.commit_block_body(&mut txn, &body, 1).is_ok());
        txn.commit();
    }

    assert_eq!(accounts.get(&address_miner1, None).balance(), policy::block_reward_at(1));

    let value1 = Coin::from(5);
    let fee1 = Coin::from(3);
    let value2 = Coin::from(7);
    let fee2 = Coin::from(11);
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
        assert!(accounts.commit_block_body(&mut txn, &body, 2).is_ok());
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
        10.into(),
        Coin::ZERO,
        1,
        NetworkId::Main
    );

    let mut body = BlockBody {
        miner: address_sender.clone(),
        extra_data: Vec::new(),
        transactions: vec![tx.clone()],
        pruned_accounts: Vec::new()
    };

    let hash1 = accounts.hash(None);
    assert_eq!(accounts.get(&address_sender, None).balance(), Coin::ZERO);
    assert_eq!(accounts.get(&address_recipient, None).balance(), Coin::ZERO);

    // Fails as address_sender does not have any funds.
    {
        let mut txn = WriteTransaction::new(&env);
        assert!(accounts.commit_block_body(&mut txn, &body, 1).is_err());
    }

    assert_eq!(accounts.get(&address_sender, None).balance(), Coin::ZERO);
    assert_eq!(accounts.get(&address_recipient, None).balance(), Coin::ZERO);
    assert_eq!(hash1, accounts.hash(None));

    // Give address_sender one block reward.
    body.transactions = Vec::new();

    {
        let mut txn = WriteTransaction::new(&env);
        assert!(accounts.commit_block_body(&mut txn, &body, 1).is_ok());
        txn.commit();
    }

    assert_eq!(accounts.get(&address_sender, None).balance(), policy::block_reward_at(1));
    assert_eq!(accounts.get(&address_recipient, None).balance(), Coin::ZERO);
    let hash2 = accounts.hash(None);
    assert_ne!(hash1, hash2);

    // Single transaction exceeding funds.
    tx.value = policy::block_reward_at(1) + Coin::from(10);
    body.transactions = vec![tx.clone()];

    {
        let mut txn = WriteTransaction::new(&env);
        assert!(accounts.commit_block_body(&mut txn, &body, 2).is_err());
    }

    assert_eq!(accounts.get(&address_sender, None).balance(), policy::block_reward_at(1));
    assert_eq!(accounts.get(&address_recipient, None).balance(), Coin::ZERO);
    assert_eq!(hash2, accounts.hash(None));

    // Multiple transactions exceeding funds.
    tx.value = Coin::from(u64::from(policy::block_reward_at(1)) / 2 + 10);
    let mut tx2 = tx.clone();
    tx2.value = tx2.value + Coin::from(10);
    body.transactions = vec![tx, tx2];

    {
        let mut txn = WriteTransaction::new(&env);
        assert!(accounts.commit_block_body(&mut txn, &body, 2).is_err());
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
        pruned_accounts: Vec::new()
    };

    // Give a block reward
    {
        let mut txn = WriteTransaction::new(&env);
        assert!(accounts.commit_block_body(&mut txn, &body, 1).is_ok());
        txn.commit();
    }

    // Create vesting contract
    let mut data: Vec<u8> = Vec::with_capacity(Address::SIZE + 4);
    address.serialize(&mut data).unwrap();
    1u32.serialize(&mut data).unwrap();
    let mut tx_create = Transaction::new_contract_creation(data, address.clone(), AccountType::Basic, AccountType::Vesting, Coin::from(100), Coin::from(0), 1, NetworkId::Dummy);
    tx_create.proof = SignatureProof::from(key_pair.public, key_pair.sign(&tx_create.serialize_content())).serialize_to_vec();
    let contract_address = tx_create.contract_creation_address();
    body.transactions = vec![tx_create.clone()];
    {
        let mut txn = WriteTransaction::new(&env);
        assert!(accounts.commit_block_body(&mut txn, &body, 2).is_ok());
        txn.commit();
    }

    // Create a block stub from it (for later use)
    let block = Block {
        header: BlockHeader {
            version: 1,
            prev_hash: Blake2bHash::from([0u8; 32]),
            interlink_hash: Blake2bHash::from([0u8; 32]),
            body_hash: body.hash(),
            accounts_hash: accounts.hash(None),
            n_bits: TargetCompact::from(1),
            height: 2,
            timestamp: 0,
            nonce: 0
        },
        interlink: BlockInterlink::new(vec![], &Blake2bHash::from([0u8; 32])),
        body: Some(body.clone()),
    };

    // Empty vesting contract without pruning it
    let mut tx_prune = Transaction::new_basic(contract_address.clone(), address.clone(), Coin::from(100), Coin::from(0), 2, NetworkId::Dummy);
    tx_prune.sender_type = AccountType::Vesting;
    tx_prune.proof = SignatureProof::from(key_pair.public, key_pair.sign(&tx_prune.serialize_content())).serialize_to_vec();
    body.transactions = vec![tx_prune.clone()];
    {
        let mut txn = WriteTransaction::new(&env);
        assert_eq!(accounts.commit_block_body(&mut txn, &body, 3), Err(AccountError::InvalidPruning));
    }

    // Now do proper pruning
    body.pruned_accounts = vec![PrunedAccount {
        address: contract_address.clone(),
        account: accounts.get(&contract_address, None)
            .with_outgoing_transaction(&tx_prune, 2).unwrap(),
    }];
    {
        let mut txn = WriteTransaction::new(&env);
        assert!(accounts.commit_block_body(&mut txn, &body, 3).is_ok());
        txn.commit();
    }

    // Check that the account was pruned correctly
    let account_after_prune = accounts.get(&contract_address, None);
    assert_eq!(account_after_prune.account_type(), AccountType::Basic);
    assert_eq!(account_after_prune.balance(), Coin::from(0));

    // Now revert pruning
    {
        let mut txn = WriteTransaction::new(&env);
        assert!(accounts.revert_block_body(&mut txn, &body, 3).is_ok());
        txn.commit();
    }

    // Check that the account was recovered correctly
    if let Account::Vesting(vesting_contract) = accounts.get(&contract_address, None) {
        assert_eq!(vesting_contract.balance, Coin::from(100));
        assert_eq!(vesting_contract.owner, address);
    }

    // New revert account
    body.pruned_accounts = Vec::new();
    body.transactions = vec![tx_create.clone()];
    {
        let mut txn = WriteTransaction::new(&env);
        assert!(accounts.revert_block(&mut txn, &block).is_ok());
        txn.commit();
    }

    // Check that the account is really gone
    let account_after_prune = accounts.get(&contract_address, None);
    assert_eq!(account_after_prune.account_type(), AccountType::Basic);
    assert_eq!(account_after_prune.balance(), Coin::from(0));
}
