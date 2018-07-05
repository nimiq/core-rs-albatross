use nimiq::consensus::base::account::Accounts;
use nimiq::consensus::base::primitive::Address;
use nimiq::consensus::base::block::BlockBody;
use nimiq::utils::db::WriteTransaction;
use nimiq::utils::db::volatile::VolatileEnvironment;
use nimiq::consensus::base::transaction::Transaction;
use nimiq::consensus::networks::NetworkId;
use nimiq::consensus::policy;

#[test]
fn it_can_commit_and_revert_a_block_body() {
    let env = VolatileEnvironment::new();
    let accounts = Accounts::new(&env);
    let address_miner = Address::from([1u8; Address::SIZE]);
    let address_recipient = Address::from([2u8; Address::SIZE]);

    let mut body = BlockBody {
        miner: address_miner.clone(),
        extra_data: Vec::new(),
        transactions: Vec::new(),
        pruned_accounts: Vec::new()
    };

    assert_eq!(accounts.get(&address_miner, None).balance(), 0);
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
        10,
        0,
        1,
        NetworkId::Main
    );
    body.transactions = vec![tx];

    assert_eq!(accounts.get(&address_recipient, None).balance(), 0);
    {
        let mut txn = WriteTransaction::new(&env);
        assert!(accounts.commit_block_body(&mut txn, &body, 2).is_ok());
        txn.commit();
    }

    assert_eq!(accounts.get(&address_recipient, None).balance(), 10);
    assert_eq!(accounts.get(&address_miner, None).balance(),
               policy::block_reward_at(1) + policy::block_reward_at(2) - 10);
    assert_ne!(hash1, accounts.hash(None));

    {
        let mut txn = WriteTransaction::new(&env);
        assert!(accounts.revert_block_body(&mut txn, &body, 2).is_ok());
        txn.commit();
    }

    assert_eq!(accounts.get(&address_recipient, None).balance(), 0);
    assert_eq!(accounts.get(&address_miner, None).balance(), policy::block_reward_at(1));
    assert_eq!(hash1, accounts.hash(None));
}

#[test]
fn it_can_deal_with_multiple_transactions_per_sender() {

}

#[test]
fn it_correctly_rewards_miners() {
    let env = VolatileEnvironment::new();
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
    assert_eq!(accounts.get(&address_miner1, None).balance(), 0);
    {
        let mut txn = WriteTransaction::new(&env);
        assert!(accounts.commit_block_body(&mut txn, &body, 1).is_ok());
        txn.commit();
    }

    assert_eq!(accounts.get(&address_miner1, None).balance(), policy::block_reward_at(1));

    let value1 = 5;
    let fee1 = 3;
    let value2 = 7;
    let fee2 = 11;
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

    assert_eq!(accounts.get(&address_miner2, None).balance(), 0);
    {
        let mut txn = WriteTransaction::new(&env);
        assert!(accounts.commit_block_body(&mut txn, &body, 2).is_ok());
        txn.commit();
    }

    assert_eq!(accounts.get(&address_miner1, None).balance(), policy::block_reward_at(2) - value1 - fee1 - value2 - fee2);
    assert_eq!(accounts.get(&address_miner2, None).balance(), policy::block_reward_at(2) + fee1 + fee2);
    assert_eq!(accounts.get(&address_recipient1, None).balance(), value1);
    assert_eq!(accounts.get(&address_recipient2, None).balance(), value2);
}

#[test]
fn it_checks_for_sufficient_funds() {
    let env = VolatileEnvironment::new();
    let accounts = Accounts::new(&env);
    let address_sender = Address::from([1u8; Address::SIZE]);
    let address_recipient = Address::from([2u8; Address::SIZE]);

    let mut tx = Transaction::new_basic(
        address_sender.clone(),
        address_recipient.clone(),
        10,
        0,
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
    assert_eq!(accounts.get(&address_sender, None).balance(), 0);
    assert_eq!(accounts.get(&address_recipient, None).balance(), 0);

    // Fails as address_sender does not have any funds.
    {
        let mut txn = WriteTransaction::new(&env);
        assert!(accounts.commit_block_body(&mut txn, &body, 1).is_err());
    }

    assert_eq!(accounts.get(&address_sender, None).balance(), 0);
    assert_eq!(accounts.get(&address_recipient, None).balance(), 0);
    assert_eq!(hash1, accounts.hash(None));

    // Give address_sender one block reward.
    body.transactions = Vec::new();

    {
        let mut txn = WriteTransaction::new(&env);
        assert!(accounts.commit_block_body(&mut txn, &body, 1).is_ok());
        txn.commit();
    }

    assert_eq!(accounts.get(&address_sender, None).balance(), policy::block_reward_at(1));
    assert_eq!(accounts.get(&address_recipient, None).balance(), 0);
    let hash2 = accounts.hash(None);
    assert_ne!(hash1, hash2);

    // Single transaction exceeding funds.
    tx.value = policy::block_reward_at(1) + 10;
    body.transactions = vec![tx.clone()];

    {
        let mut txn = WriteTransaction::new(&env);
        assert!(accounts.commit_block_body(&mut txn, &body, 2).is_err());
    }

    assert_eq!(accounts.get(&address_sender, None).balance(), policy::block_reward_at(1));
    assert_eq!(accounts.get(&address_recipient, None).balance(), 0);
    assert_eq!(hash2, accounts.hash(None));

    // Multiple transactions exceeding funds.
    tx.value = policy::block_reward_at(1) / 2 + 10;
    let mut tx2 = tx.clone();
    tx2.value += 10;
    body.transactions = vec![tx, tx2];

    {
        let mut txn = WriteTransaction::new(&env);
        assert!(accounts.commit_block_body(&mut txn, &body, 2).is_err());
    }

    assert_eq!(accounts.get(&address_sender, None).balance(), policy::block_reward_at(1));
    assert_eq!(accounts.get(&address_recipient, None).balance(), 0);
    assert_eq!(hash2, accounts.hash(None));
}

#[test]
fn it_prevents_spending_of_funds_received_in_the_same_block() {

}
