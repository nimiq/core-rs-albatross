use std::sync::Arc;

use atomic::{Atomic, Ordering};

use beserial::{Deserialize, Serialize};
use nimiq_account::{AccountError, AccountType};
use nimiq_account::Account;
use nimiq_account::PrunedAccount;
use nimiq_account::VestingContract;
use nimiq_block::{Block, BlockError, TargetCompact};
use nimiq_blockchain::{Blockchain, BlockchainEvent, PushError, PushResult};
use nimiq_database::volatile::VolatileEnvironment;
use nimiq_hash::Hash;
use nimiq_keys::{Address, KeyPair, PrivateKey};
use nimiq_network_primitives::time::NetworkTime;
use nimiq_primitives::coin::Coin;
use nimiq_primitives::networks::NetworkId;
use nimiq_transaction::{SignatureProof, Transaction};

mod nipopow;
mod transaction_proofs;

const BLOCK_2: &str = "0001264aaf8a4f9828a76c550635da078eb466306a189fcc03710bee9f649c869d120492e3986e75ac0d1466b5d6a7694c86839767a30980f8ba0d8c6e48631bc9cdd8a3eb957567d76963ad10d11e65453f763928fb9619e5f396a0906e946cce3ca7fcbb5fb2e35055de071e868381ba426a8d79d97cb48dab8345baeb9a9abb091f010000000000025ad23a98000046fe0180010000000000000000000000000000000000000000184d696e65642077697468206c6f766520627920526963687900000000";
const BLOCK_3: &str = "0001bab534467866d83060b1af0b3493dd0f97d7071b16e1562cf4b18bdf73e71ccb4aa1fea2b8cdf2a63411776c6391a7659aef4dd25317a615499c7b461e9a0405385dbed68e76f74317cc6f4cd40db832eb71b8338fad024ddbb88f9abc79f199dd6a3500aeb5479eb460afeab3363783e243a6e551536c3c01c8fca21d7afbbb1f00fddd000000035ad23a980000968102c0010000000000000000000000000000000000000000184d696e65642077697468206c6f76652062792054616d6d6f00000000";
const BLOCK_4: &str = "0001622b0536bbe764a5723f17cde03d2fa2b67a3f42f7cab082c72222eb1e48db7a607f7686d7636b500cfa620567ede30a15a12f69e22d35dd004bbdbfcaefc12520428a900c8dfb339b99aebb1d14cc4d5cebedf562aa1806f272deecbf3c5263b62534d1cda41d1a7bf70a6850c6c82936adb9b2ef66b7421ca3c55664c1417f1f00fbb7000000045ad23a9800022dc60280bab534467866d83060b1af0b3493dd0f97d7071b16e1562cf4b18bdf73e71ccb0100000000000000000000000000000000000000001b4d696e65642077697468206c6f7665206279204372697374696e6100000000";
const BLOCK_5: &str = "000184d5a44ba5ae9961837e7fb19c176a19f77b2e0655873149017351e17b622cef4aa1fea2b8cdf2a63411776c6391a7659aef4dd25317a615499c7b461e9a0405b32082f43aae5c61bf1171e85650b550bcc2b8d020365619ecaeb924c4562770cbadc05e0c4117bf975bc3d7e55d2f3a13efe1a9baf17c0b2c3c42faee9414b31f00f98c000000055ad23a9800013f5602c0010000000000000000000000000000000000000000174d696e65642077697468206c6f7665206279204174756100000000";

#[test]
fn it_can_load_a_stored_chain() {
    let env = VolatileEnvironment::new(10).unwrap();
    let block = Block::deserialize_from_vec(&hex::decode(BLOCK_2).unwrap()).unwrap();
    let hash = block.header.hash();

    {
        let blockchain = Arc::new(Blockchain::new(&env, NetworkId::Main, Arc::new(NetworkTime::new())).unwrap());
        let status = blockchain.push(block);
        assert_eq!(status, PushResult::Extended);
    }

    let blockchain = Arc::new(Blockchain::new(&env, NetworkId::Main, Arc::new(NetworkTime::new())).unwrap());
    assert_eq!(blockchain.height(), 2);
    assert_eq!(blockchain.head_hash(), hash);
}

#[test]
fn it_can_extend_the_main_chain() {
    let env = VolatileEnvironment::new(10).unwrap();
    let blockchain = Arc::new(Blockchain::new(&env, NetworkId::Main, Arc::new(NetworkTime::new())).unwrap());

    let mut block = Block::deserialize_from_vec(&hex::decode(BLOCK_2).unwrap()).unwrap();
    let mut status = blockchain.push(block);
    assert_eq!(status, PushResult::Extended);

    block = Block::deserialize_from_vec(&hex::decode(BLOCK_3).unwrap()).unwrap();
    status = blockchain.push(block);
    assert_eq!(status, PushResult::Extended);

    block = Block::deserialize_from_vec(&hex::decode(BLOCK_4).unwrap()).unwrap();
    status = blockchain.push(block);
    assert_eq!(status, PushResult::Extended);

    block = Block::deserialize_from_vec(&hex::decode(BLOCK_5).unwrap()).unwrap();
    status = blockchain.push(block);
    assert_eq!(status, PushResult::Extended);
}

#[test]
fn it_detects_known_blocks() {
    let env = VolatileEnvironment::new(10).unwrap();
    let blockchain = Arc::new(Blockchain::new(&env, NetworkId::Main, Arc::new(NetworkTime::new())).unwrap());

    let block = Block::deserialize_from_vec(&hex::decode(BLOCK_2).unwrap()).unwrap();
    let mut status = blockchain.push(block.clone());
    assert_eq!(status, PushResult::Extended);

    status = blockchain.push(block);
    assert_eq!(status, PushResult::Known);
}

#[test]
fn it_rejects_orphan_blocks() {
    let env = VolatileEnvironment::new(10).unwrap();
    let blockchain = Arc::new(Blockchain::new(&env, NetworkId::Main, Arc::new(NetworkTime::new())).unwrap());

    let block = Block::deserialize_from_vec(&hex::decode(BLOCK_3).unwrap()).unwrap();
    let status = blockchain.push(block);
    assert_eq!(status, PushResult::Orphan);
}

#[test]
fn it_rejects_intrisically_invalid_blocks() {
    let env = VolatileEnvironment::new(10).unwrap();
    let blockchain = Arc::new(Blockchain::new(&env, NetworkId::Main, Arc::new(NetworkTime::new())).unwrap());

    let mut block = Block::deserialize_from_vec(&hex::decode(BLOCK_2).unwrap()).unwrap();
    block.header.nonce = 1;
    let status = blockchain.push(block);
    assert_eq!(status, PushResult::Invalid(PushError::InvalidBlock(BlockError::InvalidPoW)));
}

#[test]
fn it_rejects_invalid_successors() {
    let env = VolatileEnvironment::new(10).unwrap();
    let blockchain = Arc::new(Blockchain::new(&env, NetworkId::Main, Arc::new(NetworkTime::new())).unwrap());

    let mut block = Block::deserialize_from_vec(&hex::decode(BLOCK_2).unwrap()).unwrap();
    block.header.timestamp = 5000;
    block.header.nonce = 54095;

    let status = blockchain.push(block);
    assert_eq!(status, PushResult::Invalid(PushError::InvalidSuccessor));
}

#[test]
fn it_rejects_blocks_with_invalid_difficulty() {
    let env = VolatileEnvironment::new(10).unwrap();
    let blockchain = Arc::new(Blockchain::new(&env, NetworkId::Main, Arc::new(NetworkTime::new())).unwrap());

    let mut block = Block::deserialize_from_vec(&hex::decode(BLOCK_2).unwrap()).unwrap();
    block.header.n_bits = 0x1f051234.into();
    block.header.nonce = 51485;

    let status = blockchain.push(block);
    assert_eq!(status, PushResult::Invalid(PushError::DifficultyMismatch));
}

#[test]
fn it_rejects_blocks_with_duplicate_transactions() {
    let keypair: KeyPair = PrivateKey::from([1u8; PrivateKey::SIZE]).into();

    let env = VolatileEnvironment::new(10).unwrap();
    let blockchain = Blockchain::new(&env, NetworkId::Main, Arc::new(NetworkTime::new())).unwrap();

    let miner = Address::from(&keypair.public);
    let block2 = crate::next_block(&blockchain)
        .with_miner(miner.clone())
        .with_nonce(34932)
        .build();

    let mut status = blockchain.push(block2);
    assert_eq!(status, PushResult::Extended);

    // Push block 3 containing a tx.
    let mut tx = Transaction::new_basic(
        miner.clone(),
        [2u8; Address::SIZE].into(),
        Coin::from_u64(10).unwrap(),
        Coin::from_u64(0).unwrap(),
        1,
        NetworkId::Main
    );
    tx.proof = SignatureProof::from(keypair.public.clone(), keypair.sign(&tx.serialize_content())).serialize_to_vec();

    let block3 = crate::next_block(&blockchain)
        .with_miner(miner)
        .with_transactions(vec![tx.clone()])
        .with_nonce(23026)
        .build();
    status = blockchain.push(block3);
    assert_eq!(status, PushResult::Extended);

    let block4 = crate::next_block(&blockchain)
        .with_transactions(vec![tx])
        .with_nonce(6471)
        .build();
    status = blockchain.push(block4);
    assert_eq!(status, PushResult::Invalid(PushError::DuplicateTransaction));
}

#[test]
fn it_rejects_blocks_if_body_cannot_be_applied() {
    let keypair: KeyPair = PrivateKey::from([1u8; PrivateKey::SIZE]).into();

    let env = VolatileEnvironment::new(10).unwrap();
    let blockchain = Blockchain::new(&env, NetworkId::Main, Arc::new(NetworkTime::new())).unwrap();

    let miner = Address::from(&keypair.public);
    let block2 = crate::next_block(&blockchain)
        .with_miner(miner.clone())
        .with_nonce(34932)
        .build();

    let mut status = blockchain.push(block2);
    assert_eq!(status, PushResult::Extended);

    // Tx exceeding funds
    let mut tx = Transaction::new_basic(
        miner.clone(),
        [2u8; Address::SIZE].into(),
        Coin::from_u64(1000000000).unwrap(),
        Coin::from_u64(0).unwrap(),
        1,
        NetworkId::Main
    );
    tx.proof = SignatureProof::from(keypair.public.clone(), keypair.sign(&tx.serialize_content())).serialize_to_vec();

    let mut block3 = crate::next_block(&blockchain)
        .with_transactions(vec![tx.clone()])
        .with_nonce(31302)
        .build();
    status = blockchain.push(block3);
    assert_eq!(status, PushResult::Invalid(PushError::AccountsError(AccountError::InsufficientFunds { needed: Coin::from_u64(1000000000).unwrap(), balance: Coin::from_u64(440597429).unwrap() })));

    // Tx with wrong sender type
    tx = Transaction::new_basic(
        miner.clone(),
        [2u8; Address::SIZE].into(),
        Coin::from_u64(1000).unwrap(),
        Coin::from_u64(0).unwrap(),
        1,
        NetworkId::Main
    );
    tx.sender_type = AccountType::Vesting;
    tx.proof = SignatureProof::from(keypair.public.clone(), keypair.sign(&tx.serialize_content())).serialize_to_vec();

    block3 = crate::next_block(&blockchain)
        .with_transactions(vec![tx.clone()])
        .with_nonce(127678)
        .build();
    status = blockchain.push(block3);
    assert_eq!(status, PushResult::Invalid(PushError::AccountsError(AccountError::TypeMismatch { expected: AccountType::Basic, got: AccountType::Vesting })));
}

#[test]
fn it_detects_fork_blocks() {
    let env = VolatileEnvironment::new(10).unwrap();
    let blockchain = Blockchain::new(&env, NetworkId::Main, Arc::new(NetworkTime::new())).unwrap();

    let mut block = crate::next_block(&blockchain)
        .with_nonce(83054)
        .build();
    assert_eq!(blockchain.push(block), PushResult::Extended);

    block = Block::deserialize_from_vec(&hex::decode(BLOCK_2).unwrap()).unwrap();
    assert_eq!(blockchain.push(block), PushResult::Forked);
}

#[test]
fn it_rebranches_to_the_harder_chain() {
    let env = VolatileEnvironment::new(10).unwrap();
    let blockchain = Blockchain::new(&env, NetworkId::Main, Arc::new(NetworkTime::new())).unwrap();

    let block1_2 = crate::next_block(&blockchain)
        .with_nonce(83054)
        .build();
    assert_eq!(blockchain.push(block1_2.clone()), PushResult::Extended);

    let block1_3 = crate::next_block(&blockchain)
        .with_nonce(23192)
        .build();
    assert_eq!(blockchain.push(block1_3.clone()), PushResult::Extended);

    let block1_4 = crate::next_block(&blockchain)
        .with_nonce(39719)
        .build();

    let block2_2 = Block::deserialize_from_vec(&hex::decode(BLOCK_2).unwrap()).unwrap();
    assert_eq!(blockchain.push(block2_2.clone()), PushResult::Forked);

    let block2_3 = Block::deserialize_from_vec(&hex::decode(BLOCK_3).unwrap()).unwrap();
    assert_eq!(blockchain.push(block2_3.clone()), PushResult::Rebranched);

    assert_eq!(blockchain.push(block1_4.clone()), PushResult::Rebranched);

    let block2_4 = Block::deserialize_from_vec(&hex::decode(BLOCK_4).unwrap()).unwrap();

    let listener_called = Arc::new(Atomic::new(false));
    let reverted_blocks = Arc::new(vec![(block1_2.header.hash(), block1_2), (block1_3.header.hash(), block1_3), (block1_4.header.hash(), block1_4)]);
    let adopted_blocks = Arc::new(vec![(block2_2.header.hash(), block2_2), (block2_3.header.hash(), block2_3), (block2_4.header.hash(), block2_4.clone())]);
    let listener_called1 = listener_called.clone();
    blockchain.notifier.write().register(move |e: &BlockchainEvent| {
        assert_eq!(*e, BlockchainEvent::Rebranched((*reverted_blocks).clone(), (*adopted_blocks).clone()));
        listener_called1.store(true, Ordering::Relaxed);
    });

    assert_eq!(blockchain.push(block2_4), PushResult::Rebranched);
    assert!(listener_called.load(Ordering::Relaxed));
}

#[test]
fn it_deletes_invalid_forks() {
    let env = VolatileEnvironment::new(10).unwrap();
    let blockchain = Blockchain::new(&env, NetworkId::Main, Arc::new(NetworkTime::new())).unwrap();

    // Create valid chain
    let block1_2 = crate::next_block(&blockchain)
        .with_nonce(83054)
        .build();
    assert_eq!(blockchain.push(block1_2), PushResult::Extended);
    let block1_3 = crate::next_block(&blockchain)
        .with_nonce(23192)
        .build();
    assert_eq!(blockchain.push(block1_3), PushResult::Extended);
    let block1_4 = crate::next_block(&blockchain)
        .with_nonce(39719)
        .build();
    assert_eq!(blockchain.push(block1_4), PushResult::Extended);

    // Create fork with invalid pruned accounts
    let fork_env = VolatileEnvironment::new(10).unwrap();
    let fork = Blockchain::new(&fork_env, NetworkId::Main, Arc::new(NetworkTime::new())).unwrap();
    let pruned_account = PrunedAccount {
        address: [1u8; Address::SIZE].into(),
        account: Account::Vesting(VestingContract::new(Coin::from_u64_unchecked(0), [2u8; Address::SIZE].into(), 0, 500, Coin::from_u64_unchecked(2), Coin::from_u64_unchecked(200)))
    };
    let block2_2 = crate::next_block(&fork)
        .with_height(2)
        .with_pruned_accounts(vec![pruned_account])
        .with_timestamp(fork.head().header.timestamp + 1)
        .with_nonce(151483)
        .build();
    let hash2_2 = block2_2.header.hash();
    let nbits2_3 = TargetCompact::from(0x1f00fde6);
    let interlink2_3 = block2_2.get_next_interlink(&nbits2_3.into());
    assert_eq!(blockchain.push(block2_2), PushResult::Forked);
    assert!(blockchain.get_block(&hash2_2, true, false).is_some());

    let block2_3 = crate::next_block(&fork)
        .with_prev_hash(hash2_2.clone())
        .with_height(3)
        .with_nbits(nbits2_3)
        .with_interlink(interlink2_3)
        .with_timestamp(fork.head().header.timestamp + 2)
        .with_nonce(105711)
        .build();
    let hash2_3 = block2_3.header.hash();
    let nbits2_4 = TargetCompact::from(0x1f00fbc9);
    let interlink2_4 = block2_3.get_next_interlink(&nbits2_4.into());
    assert_eq!(blockchain.push(block2_3), PushResult::Forked);
    assert!(blockchain.get_block(&hash2_3, true, false).is_some());

    let block2_4 = crate::next_block(&fork)
        .with_prev_hash(hash2_3.clone())
        .with_height(4)
        .with_nbits(nbits2_4)
        .with_interlink(interlink2_4)
        .with_timestamp(fork.head().header.timestamp + 3)
        .with_nonce(21362)
        .build();
    let hash2_4 = block2_4.header.hash();
    assert_eq!(blockchain.push(block2_4), PushResult::Invalid(PushError::InvalidFork));
    assert!(blockchain.get_block(&hash2_2, true, false).is_none());
    assert!(blockchain.get_block(&hash2_3, true, false).is_none());
    assert!(blockchain.get_block(&hash2_4, true, false).is_none());
}
