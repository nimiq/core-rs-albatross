use nimiq_block::{Block, BlockBody, BlockHeader, BlockInterlink, TargetCompact};
use nimiq_blockchain::transaction_store::*;
use nimiq_database::volatile::VolatileEnvironment;
use nimiq_database::WriteTransaction;
use nimiq_hash::{Blake2bHash, Hash};
use nimiq_keys::Address;
use nimiq_network_primitives::networks::get_network_info;
use nimiq_primitives::coin::Coin;
use nimiq_primitives::networks::NetworkId;
use nimiq_transaction::Transaction;

fn create_transactions() -> (Transaction, Transaction, Transaction) {
    // Prepare three transactions.
    let tx1 = Transaction::new_basic(
        [1u8; Address::SIZE].into(),
        [2u8; Address::SIZE].into(),
        Coin::from_u64(10).unwrap(),
        Coin::from_u64(0).unwrap(),
        1,
        NetworkId::Main
    );
    let tx2 = Transaction::new_basic(
        [4u8; Address::SIZE].into(),
        [2u8; Address::SIZE].into(),
        Coin::from_u64(12).unwrap(),
        Coin::from_u64(0).unwrap(),
        1,
        NetworkId::Main
    );
    let tx3 = Transaction::new_basic(
        [1u8; Address::SIZE].into(),
        [5u8; Address::SIZE].into(),
        Coin::from_u64(14).unwrap(),
        Coin::from_u64(0).unwrap(),
        1,
        NetworkId::Main
    );
    (tx1, tx2, tx3)
}

fn create_block(transactions: Vec<Transaction>, height: u32) -> Block {
    let body = BlockBody {
        miner: Address::default(),
        extra_data: Vec::new(),
        transactions,
        pruned_accounts: Vec::new(),
    };
    let interlink = BlockInterlink::default();

    Block {
        header: BlockHeader {
            version: 0,
            prev_hash: Blake2bHash::default(),
            interlink_hash: interlink.hash(Blake2bHash::default()),
            body_hash: body.hash(),
            accounts_hash: Blake2bHash::default(),
            n_bits: TargetCompact::default(),
            height,
            timestamp: 0,
            nonce: 0,
        },
        interlink,
        body: Some(body)
    }
}

#[test]
fn it_can_store_and_remove_transactions() {
    let env = VolatileEnvironment::new(4).unwrap();
    let store = TransactionStore::new(&env);

    let (tx1, tx2, tx3) = create_transactions();

    // Block 1 (empty)
    let block1 = get_network_info(NetworkId::Main).unwrap().genesis_block.clone();
    let infos1 = TransactionInfo::from_block(&block1);
    assert_eq!(infos1.len(), 0);

    {
        let mut txn = WriteTransaction::new(&env);
        store.put(&block1, &mut txn);
        txn.commit();
    }

    assert!(store.get_by_hash(&tx1.hash(), None).is_none());
    assert_eq!(store.get_by_sender(&tx1.sender, 1, None).len(), 0);

    // Block 2 (2 txs)
    let block2 = create_block(vec![tx1.clone(), tx2.clone()], 2);
    let infos2 = TransactionInfo::from_block(&block2);
    assert_eq!(infos2.len(), 2);

    {
        let mut txn = WriteTransaction::new(&env);
        store.put(&block2, &mut txn);
        txn.commit();
    }

    let info = store.get_by_hash(&tx1.hash(), None);
    assert!(info.is_some());
    assert_eq!(info.as_ref().unwrap().index, 0);
    assert_eq!(info.as_ref().unwrap().block_height, 2);
    assert_eq!(store.get_by_sender(&tx2.sender, 1, None).len(), 1);
    assert_eq!(store.get_by_recipient(&tx2.recipient, 5, None).len(), 2);
    assert!(store.get_by_hash(&tx3.hash(), None).is_none());

    // Remove Block 2
    {
        let mut txn = WriteTransaction::new(&env);
        store.remove(&block2, &mut txn);
        txn.commit();
    }

    assert!(store.get_by_hash(&tx1.hash(), None).is_none());
    assert_eq!(store.get_by_sender(&tx1.sender, 1, None).len(), 0);
}

#[test]
fn it_can_retrieve_by_sender() {
    let env = VolatileEnvironment::new(4).unwrap();
    let store = TransactionStore::new(&env);

    let (tx1, tx2, tx3) = create_transactions();
    // Block (3 txs)
    let block = create_block(vec![tx1.clone(), tx2.clone(), tx3.clone()], 2);

    {
        let mut txn = WriteTransaction::new(&env);
        store.put(&block, &mut txn);
        txn.commit();
    }

    let txs = store.get_by_sender(&tx1.sender, 5, None);
    assert_eq!(txs.len(), 2);
    assert_eq!(txs[0].transaction_hash, tx3.hash());
    assert_eq!(txs[1].transaction_hash, tx1.hash());

    let txs = store.get_by_sender(&tx2.sender, 5, None);
    assert_eq!(txs.len(), 1);
    assert_eq!(txs[0].transaction_hash, tx2.hash());
}

#[test]
fn it_can_retrieve_by_recipient() {
    let env = VolatileEnvironment::new(4).unwrap();
    let store = TransactionStore::new(&env);

    let (tx1, tx2, tx3) = create_transactions();
    // Block (3 txs)
    let block = create_block(vec![tx1.clone(), tx2.clone(), tx3.clone()], 2);

    {
        let mut txn = WriteTransaction::new(&env);
        store.put(&block, &mut txn);
        txn.commit();
    }

    let txs = store.get_by_recipient(&tx1.recipient, 5, None);
    assert_eq!(txs.len(), 2);
    assert_eq!(txs[0].transaction_hash, tx2.hash());
    assert_eq!(txs[1].transaction_hash, tx1.hash());

    let txs = store.get_by_recipient(&tx3.recipient, 5, None);
    assert_eq!(txs.len(), 1);
    assert_eq!(txs[0].transaction_hash, tx3.hash());
}

#[test]
fn it_can_retrieve_by_hash() {
    let env = VolatileEnvironment::new(4).unwrap();
    let store = TransactionStore::new(&env);

    let (tx1, tx2, tx3) = create_transactions();
    // Block (3 txs)
    let block = create_block(vec![tx1.clone(), tx2.clone(), tx3.clone()], 2);

    {
        let mut txn = WriteTransaction::new(&env);
        store.put(&block, &mut txn);
        txn.commit();
    }

    let info = store.get_by_hash(&tx2.hash(), None);
    assert!(info.is_some());
    assert_eq!(info.unwrap().transaction_hash, tx2.hash());
}

#[test]
fn it_can_rebranch() {
    let env = VolatileEnvironment::new(4).unwrap();
    let store = TransactionStore::new(&env);

    let (tx1, tx2, tx3) = create_transactions();
    // Block (3 txs)
    let block = create_block(vec![tx1.clone(), tx2.clone(), tx3.clone()], 2);
    let rebranched_block1 = create_block(vec![tx3.clone()], 2);
    let rebranched_block2 = create_block(vec![tx1.clone()], 3);

    {
        let mut txn = WriteTransaction::new(&env);
        store.put(&block, &mut txn);
        store.remove(&block, &mut txn);
        store.put(&rebranched_block1, &mut txn);
        store.put(&rebranched_block2, &mut txn);
        txn.commit();
    }

    let info = store.get_by_hash(&tx1.hash(), None);
    assert!(info.is_some());
    let info = info.unwrap();
    assert_eq!(info.block_height, 3);
    assert_eq!(info.block_hash, rebranched_block2.header.hash());

    let info = store.get_by_hash(&tx2.hash(), None);
    assert!(info.is_none());

    let info = store.get_by_hash(&tx3.hash(), None);
    assert!(info.is_some());
    let info = info.unwrap();
    assert_eq!(info.block_height, 2);
    assert_eq!(info.block_hash, rebranched_block1.header.hash());
}
