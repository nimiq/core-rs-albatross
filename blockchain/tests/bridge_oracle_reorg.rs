//! Micro-block reorgs across bridge and oracle transactions.
//!
//! Bridge and oracle state lives inside consensus, so a revert that does not restore it exactly is
//! not a lost transfer but a chain fork: two honest nodes that reached the same chain by different
//! routes end up with different accounts-tree roots. These tests drive real micro-block rebranches
//! — not account-layer `revert_*` calls — through releases, locks, ring-wrapping oracle updates and
//! the contract creations themselves. Each scenario runs four nodes: one that applies the doomed
//! blocks and has to rebranch, one that authors the winning fork and never saw them, one that
//! mirrors the first, and a late joiner that only ever receives the canonical chain in order. All
//! four must agree on the head and the accounts root at every step, and the accounts a reorg puts
//! back must match what was there before, byte for byte.

use nimiq_account::{Account, OracleContract};
use nimiq_block::Block;
use nimiq_blockchain_interface::{AbstractBlockchain, PushResult};
use nimiq_genesis::NetworkId;
use nimiq_hash::{Blake2bHash, Blake2bHasher, HashOutput, Hasher};
use nimiq_keys::{Address, KeyPair, PrivateKey};
use nimiq_primitives::{account::AccountType, coin::Coin, policy::upgrades};
use nimiq_serde::{Deserialize, Serialize};
use nimiq_test_log::test;
use nimiq_test_utils::{block_production::TemporaryBlockProducer, blockchain::REWARD_KEY};
use nimiq_transaction::{
    account::{
        bridge_contract::OutgoingBridgeTransactionData,
        htlc_contract::{AnyHash, AnyHash32},
        oracle_contract::IncomingOracleTransactionData,
    },
    bridge_contract::{
        AddressFormat, AnyMerkleProof, ChainConfig, Endianness, OutgoingTransaction, ValidationOp,
        ValidationProgram,
    },
    SignatureProof, Transaction,
};
use nimiq_transaction_builder::TransactionBuilder;
use nimiq_utils::merkle::MerklePath;

const SOURCE_CHAIN_ID: u32 = 1;
const ORACLE_DEPOSIT: u64 = 1_000;
const BRIDGE_DEPOSIT: u64 = 10_000;
const LOCK_AMOUNT: u64 = 1_000;
const RELEASE_AMOUNT: u64 = 500;
const BURN_BLOCK_HEIGHT: u32 = 42;

/// The oracle owner. Fixed rather than generated so every run builds the same transactions.
const ORACLE_OWNER_KEY: &str = "9d5bd02379e7e45cf515c788048f5cf3c454ffabd3e83bd1d7667716c325c3c0";
/// Whoever submits the burn proof. Releases are permissionless and this one pays no fee, so the
/// key needs no funds.
const SUBMITTER_KEY: &str = "0f0e0d0c0b0a09080706050403020100ffeeddccbbaa99887766554433221100";

// ---------------------------------------------------------------------------------------------
// Keys, config, payloads
// ---------------------------------------------------------------------------------------------

fn key(hex_key: &str) -> KeyPair {
    KeyPair::from(PrivateKey::deserialize_from_vec(&hex::decode(hex_key).unwrap()).unwrap())
}

/// The genesis account that funds everything: contract deposits, locks and the relayer's updates.
fn funds() -> KeyPair {
    key(REWARD_KEY)
}

fn funds_address() -> Address {
    Address::from(&funds().public)
}

/// Little-endian fixed-offset program: target_address [0..20], amount [20..28],
/// target_nonce [28..36], burn_block_height [36..40], target_chain_id [40..44].
fn chain_config() -> ChainConfig {
    ChainConfig {
        chain_id: SOURCE_CHAIN_ID,
        hash_function: AnyHash::Blake2b(AnyHash32::default()),
        address_format: AddressFormat::Nimiq,
        endianness: Endianness::LittleEndian,
        block_time: std::time::Duration::from_secs(60),
        validation_program: ValidationProgram::new(vec![
            ValidationOp::PushConst(0),
            ValidationOp::LoadAddress,
            ValidationOp::Store("target_address".to_string()),
            ValidationOp::PushConst(20),
            ValidationOp::LoadU64(Endianness::LittleEndian),
            ValidationOp::Store("amount".to_string()),
            ValidationOp::PushConst(28),
            ValidationOp::LoadU64(Endianness::LittleEndian),
            ValidationOp::Store("target_nonce".to_string()),
            ValidationOp::PushConst(36),
            ValidationOp::LoadU32(Endianness::LittleEndian),
            ValidationOp::Store("burn_block_height".to_string()),
            ValidationOp::PushConst(40),
            ValidationOp::LoadU32(Endianness::LittleEndian),
            ValidationOp::Store("target_chain_id".to_string()),
        ]),
        max_proof_depth: 64,
    }
}

fn burn_data(target: &Address, amount: u64, nonce: u64) -> Vec<u8> {
    let mut data = Vec::with_capacity(44);
    data.extend_from_slice(target.as_bytes());
    data.extend_from_slice(&amount.to_le_bytes());
    data.extend_from_slice(&nonce.to_le_bytes());
    data.extend_from_slice(&BURN_BLOCK_HEIGHT.to_le_bytes());
    data.extend_from_slice(&SOURCE_CHAIN_ID.to_le_bytes());
    data
}

fn blake2b(data: &[u8]) -> AnyHash {
    let hash = Blake2bHasher::default().digest(data);
    AnyHash::Blake2b(AnyHash32::from(
        <[u8; 32]>::try_from(hash.as_bytes()).unwrap(),
    ))
}

// ---------------------------------------------------------------------------------------------
// Transactions
// ---------------------------------------------------------------------------------------------

fn create_oracle_tx(owner: &KeyPair, hash_count: u16, validity_start_height: u32) -> Transaction {
    TransactionBuilder::new_create_oracle(
        &funds(),
        Address::from(&owner.public),
        hash_count,
        Coin::from_u64_unchecked(ORACLE_DEPOSIT),
        Coin::ZERO,
        validity_start_height,
        NetworkId::UnitAlbatross,
    )
    .expect("a valid oracle creation")
}

fn create_bridge_tx(owner: &KeyPair, oracle: &Address, validity_start_height: u32) -> Transaction {
    TransactionBuilder::new_create_bridge(
        &funds(),
        Address::from(&owner.public),
        oracle.clone(),
        SOURCE_CHAIN_ID,
        chain_config(),
        Coin::from_u64_unchecked(BRIDGE_DEPOSIT),
        Coin::ZERO,
        validity_start_height,
        NetworkId::UnitAlbatross,
    )
    .expect("a valid bridge creation")
}

/// An oracle `Update` as the relayer submits it: the relayer's own account is the sender and pays
/// the fee, the owner authorizes the hashes with a signature carried inside the data. (A
/// self-addressed oracle→oracle transaction, which the account-layer tests use, is refused by
/// `Transaction::verify` at the block level: sender and recipient may not coincide.)
fn oracle_update_tx(
    oracle: &Address,
    owner: &KeyPair,
    hashes: Vec<AnyHash>,
    validity_start_height: u32,
) -> Transaction {
    let relayer = funds();
    let data = IncomingOracleTransactionData::Update {
        hashes,
        proof: SignatureProof::default(),
    };
    let mut tx = Transaction::new_signaling(
        Address::from(&relayer.public),
        AccountType::Basic,
        oracle.clone(),
        AccountType::Oracle,
        Coin::ZERO,
        data.serialize_to_vec(),
        validity_start_height,
        NetworkId::UnitAlbatross,
    );
    let owner_proof =
        SignatureProof::from_ed25519(owner.public, owner.sign(&tx.serialize_content()));
    tx.recipient_data =
        IncomingOracleTransactionData::set_signature_on_data(&tx.recipient_data, owner_proof)
            .expect("the owner's signature fits the data");
    tx.proof = SignatureProof::from_ed25519(relayer.public, relayer.sign(&tx.serialize_content()))
        .serialize_to_vec();
    tx
}

/// A user locking NIM into the bridge: a plain transfer whose recipient is the bridge contract.
fn lock_tx(bridge: &Address, amount: u64, validity_start_height: u32) -> Transaction {
    let sender = funds();
    let mut tx = Transaction::new_extended(
        Address::from(&sender.public),
        AccountType::Basic,
        vec![],
        bridge.clone(),
        AccountType::Bridge,
        vec![],
        Coin::from_u64_unchecked(amount),
        Coin::ZERO,
        validity_start_height,
        NetworkId::UnitAlbatross,
    );
    tx.proof = SignatureProof::from_ed25519(sender.public, sender.sign(&tx.serialize_content()))
        .serialize_to_vec();
    tx
}

/// A release: the burn proof against oracle state `oracle_state_index`, signed by `submitter`.
fn release_tx(
    bridge: &Address,
    target: &Address,
    amount: u64,
    burn: Vec<u8>,
    oracle_state_index: u64,
    submitter: &KeyPair,
    validity_start_height: u32,
) -> Transaction {
    let mut bridge_data = OutgoingBridgeTransactionData {
        burn_proof: OutgoingTransaction {
            burn_transaction_data: burn,
            merkle_proof: AnyMerkleProof::Blake2bPath(MerklePath::empty()),
            oracle_state_index,
        },
        proof: SignatureProof::default(),
    };
    let mut tx = Transaction::new_extended(
        bridge.clone(),
        AccountType::Bridge,
        bridge_data.serialize_to_vec(),
        target.clone(),
        AccountType::Basic,
        vec![],
        Coin::from_u64_unchecked(amount),
        Coin::ZERO,
        validity_start_height,
        NetworkId::UnitAlbatross,
    );
    bridge_data.set_signature(SignatureProof::from_ed25519(
        submitter.public,
        submitter.sign(&tx.serialize_content()),
    ));
    tx.sender_data = bridge_data.serialize_to_vec();
    tx
}

// ---------------------------------------------------------------------------------------------
// Nodes
// ---------------------------------------------------------------------------------------------

fn node() -> TemporaryBlockProducer {
    TemporaryBlockProducer::new_with_protocol_version(upgrades::v3::BRIDGE_ORACLE_CONTRACTS)
}

fn height(node: &TemporaryBlockProducer) -> u32 {
    node.blockchain.read().block_number()
}

fn head(node: &TemporaryBlockProducer) -> Blake2bHash {
    node.blockchain.read().head_hash()
}

fn root(node: &TemporaryBlockProducer) -> Blake2bHash {
    node.blockchain
        .read()
        .state
        .accounts
        .get_root_hash_assert(None)
}

fn account(node: &TemporaryBlockProducer, address: &Address) -> Option<Account> {
    node.blockchain.read().get_account_if_complete(address)
}

/// The account exactly as it sits in the tree.
fn account_bytes(node: &TemporaryBlockProducer, address: &Address) -> Vec<u8> {
    account(node, address)
        .expect("the accounts tree is complete")
        .serialize_to_vec()
}

fn luna(node: &TemporaryBlockProducer, address: &Address) -> u64 {
    account(node, address)
        .map(|account| u64::from(account.balance()))
        .unwrap_or(0)
}

fn oracle_contract(node: &TemporaryBlockProducer, address: &Address) -> OracleContract {
    match account(node, address) {
        Some(Account::Oracle(oracle)) => oracle,
        other => panic!("expected an oracle contract at {address}, found {other:?}"),
    }
}

fn extend(label: &str, node: &TemporaryBlockProducer, block: &Block) {
    assert_eq!(
        node.push(block.clone()),
        Ok(PushResult::Extended),
        "{label} must extend its chain with {block}"
    );
}

fn rebranch(label: &str, node: &TemporaryBlockProducer, block: &Block) {
    assert_eq!(
        node.push(block.clone()),
        Ok(PushResult::Rebranched),
        "{label} must rebranch onto {block}"
    );
}

fn assert_same_chain(nodes: &[&TemporaryBlockProducer], context: &str) {
    let (reference_height, reference_head, reference_root) =
        (height(nodes[0]), head(nodes[0]), root(nodes[0]));
    for (i, node) in nodes.iter().enumerate().skip(1) {
        assert_eq!(height(node), reference_height, "{context}: node {i} height");
        assert_eq!(head(node), reference_head, "{context}: node {i} head");
        assert_eq!(
            root(node),
            reference_root,
            "{context}: node {i} accounts-tree root"
        );
    }
}

/// Four nodes that will end up on the same chain by different routes.
struct World {
    /// Applies the doomed blocks and has to rebranch away from them.
    main: TemporaryBlockProducer,
    /// Never sees the doomed blocks; authors the fork that wins.
    fork: TemporaryBlockProducer,
    /// Mirrors `main` block for block, including the rebranch.
    follower: TemporaryBlockProducer,
    /// Only ever receives the canonical chain, in order.
    late: TemporaryBlockProducer,
    /// Validity start height shared by every transaction. The unit-test validity window is two
    /// blocks, but a transaction is also valid for a whole batch *before* its start height, so
    /// dating them ahead lets the same signed transaction be reverted by a reorg and re-included
    /// a few blocks later.
    validity_start_height: u32,
}

impl World {
    fn new() -> Self {
        let main = node();
        let validity_start_height = height(&main) + 16;
        World {
            main,
            fork: node(),
            follower: node(),
            late: node(),
            validity_start_height,
        }
    }

    fn all(&self) -> [&TemporaryBlockProducer; 4] {
        [&self.main, &self.fork, &self.follower, &self.late]
    }

    /// A block on the common prefix: produced by `main`, applied everywhere.
    fn common_block(&self, transactions: Vec<Transaction>) -> Block {
        let block = self.main.next_block_with_txs(vec![], false, transactions);
        extend("fork", &self.fork, &block);
        extend("follower", &self.follower, &block);
        extend("late joiner", &self.late, &block);
        block
    }

    /// A block that only the doomed branch sees.
    fn doomed_block(&self, transactions: Vec<Transaction>) -> Block {
        let block = self.main.next_block_with_txs(vec![], false, transactions);
        extend("follower", &self.follower, &block);
        block
    }

    /// A block on the canonical chain after the reorg: produced by `fork`, applied everywhere.
    fn canonical_block(&self, transactions: Vec<Transaction>) -> Block {
        let block = self.fork.next_block_with_txs(vec![], false, transactions);
        extend("main", &self.main, &block);
        extend("follower", &self.follower, &block);
        extend("late joiner", &self.late, &block);
        block
    }

    /// The fork overtakes the doomed branch. Its first block is a skip block at the divergence
    /// height — the validators attesting that the slot owner was offline — which beats the
    /// doomed branch outright however long it grew; the fork then extends past the doomed tip.
    fn overtake(&self, doomed_blocks: usize) {
        let skip_block = self.fork.next_block(vec![], true);
        rebranch("main", &self.main, &skip_block);
        rebranch("follower", &self.follower, &skip_block);
        extend("late joiner", &self.late, &skip_block);
        for _ in 0..doomed_blocks {
            self.canonical_block(vec![]);
        }
        assert_same_chain(&self.all(), "after the reorg");
    }
}

// ---------------------------------------------------------------------------------------------
// Scenarios
// ---------------------------------------------------------------------------------------------

/// A reorg drops a block carrying a lock and a nonce-1 release. The bridge account must come back
/// byte for byte (balance and `transaction_count`), the payout must vanish, the lock must be
/// refunded — and the nonce ledger must be empty again, which is proved by the *same* release,
/// nonce 1 and all, succeeding on the canonical chain afterwards. Had the entry survived, that
/// release would land as a failed transaction and the four nodes' bridge bytes would diverge from
/// the pre-reorg run.
#[test]
fn a_reorg_that_drops_a_release_restores_the_bridge_and_its_nonce_ledger() {
    let w = World::new();
    let owner = key(ORACLE_OWNER_KEY);
    let submitter = key(SUBMITTER_KEY);
    let target = Address::from([0xAAu8; 20]);

    let create_oracle = create_oracle_tx(&owner, 10, w.validity_start_height);
    let oracle = create_oracle.recipient.clone();
    let create_bridge = create_bridge_tx(&owner, &oracle, w.validity_start_height);
    let bridge = create_bridge.recipient.clone();
    let burn = burn_data(&target, RELEASE_AMOUNT, 1);

    // Common prefix: the contracts exist and the relayer has published the burn's root.
    w.common_block(vec![create_oracle]);
    w.common_block(vec![create_bridge]);
    w.common_block(vec![oracle_update_tx(
        &oracle,
        &owner,
        vec![blake2b(&burn)],
        w.validity_start_height,
    )]);
    let bridge_before = account_bytes(&w.main, &bridge);
    let oracle_before = account_bytes(&w.main, &oracle);
    let funds_before = luna(&w.main, &funds_address());
    assert_eq!(luna(&w.main, &target), 0);

    // The doomed block: a user locks NIM and the submitter releases the burn.
    let lock = lock_tx(&bridge, LOCK_AMOUNT, w.validity_start_height);
    let release = release_tx(
        &bridge,
        &target,
        RELEASE_AMOUNT,
        burn,
        0,
        &submitter,
        w.validity_start_height,
    );
    w.doomed_block(vec![lock.clone(), release.clone()]);

    let bridge_after = account_bytes(&w.main, &bridge);
    assert_ne!(bridge_after, bridge_before);
    assert_eq!(
        luna(&w.main, &bridge),
        BRIDGE_DEPOSIT + LOCK_AMOUNT - RELEASE_AMOUNT
    );
    assert_eq!(luna(&w.main, &target), RELEASE_AMOUNT);
    match account(&w.main, &bridge) {
        Some(Account::Bridge(contract)) => assert_eq!(contract.transaction_count, 2),
        other => panic!("expected the bridge contract, found {other:?}"),
    }
    assert_same_chain(&[&w.main, &w.follower], "doomed branch");

    w.overtake(1);

    assert_eq!(
        account_bytes(&w.main, &bridge),
        bridge_before,
        "the bridge account must be restored byte for byte"
    );
    assert_eq!(
        account_bytes(&w.main, &oracle),
        oracle_before,
        "the oracle must be untouched"
    );
    assert_eq!(luna(&w.main, &target), 0, "the payout must be gone");
    assert_eq!(
        luna(&w.main, &funds_address()),
        funds_before,
        "the lock must be refunded"
    );

    // The identical lock and release on the canonical chain: nonce 1 is free again, and the
    // outcome is the same bytes the doomed branch had produced.
    w.canonical_block(vec![lock, release]);
    assert_eq!(
        account_bytes(&w.main, &bridge),
        bridge_after,
        "replaying the same lock and release must reach the same bridge bytes"
    );
    assert_eq!(luna(&w.main, &target), RELEASE_AMOUNT);
    assert_same_chain(&w.all(), "after the replay");
}

/// A reorg drops two oracle updates, each of which had wrapped a two-slot ring buffer and evicted
/// an entry. The ring must come back exactly as it was when full — evicted entries included — and
/// replaying the same updates on the canonical chain must reach the same bytes the doomed branch
/// had, on all four nodes.
#[test]
fn a_reorg_that_drops_oracle_updates_restores_the_ring_buffer_across_a_wrap() {
    let w = World::new();
    let owner = key(ORACLE_OWNER_KEY);
    let leaf = |tag: u8| blake2b(&[tag; 32]);

    let create_oracle = create_oracle_tx(&owner, 2, w.validity_start_height);
    let oracle = create_oracle.recipient.clone();
    let update =
        |hashes: Vec<AnyHash>| oracle_update_tx(&oracle, &owner, hashes, w.validity_start_height);

    // Common prefix: the ring is exactly full.
    w.common_block(vec![create_oracle]);
    w.common_block(vec![update(vec![leaf(1)])]);
    w.common_block(vec![update(vec![leaf(2)])]);
    let ring_full = oracle_contract(&w.main, &oracle);
    assert_eq!(ring_full.hashes.len(), 2);
    assert_eq!(ring_full.latest_index, Some(1));
    let oracle_full = account_bytes(&w.main, &oracle);

    // Doomed: two updates that each wrap the ring and evict its oldest entry.
    let wrap_once = update(vec![leaf(3)]);
    let wrap_twice = update(vec![leaf(4), leaf(5)]);
    w.doomed_block(vec![wrap_once.clone()]);
    w.doomed_block(vec![wrap_twice.clone()]);
    let oracle_wrapped = account_bytes(&w.main, &oracle);
    assert_ne!(oracle_wrapped, oracle_full);
    assert_eq!(oracle_contract(&w.main, &oracle).hashes.len(), 2);
    assert_same_chain(&[&w.main, &w.follower], "doomed branch");

    w.overtake(2);

    assert_eq!(
        account_bytes(&w.main, &oracle),
        oracle_full,
        "the ring must be restored byte for byte, evicted entries included"
    );

    w.canonical_block(vec![wrap_once]);
    w.canonical_block(vec![wrap_twice]);
    assert_eq!(
        account_bytes(&w.main, &oracle),
        oracle_wrapped,
        "replaying the same wraps must reach the same ring"
    );
    assert_same_chain(&w.all(), "after the replay");
}

/// A reorg drops the blocks that created the oracle and the bridge. Both accounts must disappear
/// and the deposits must return to the creator, and re-creating them on the canonical chain must
/// reproduce the very same accounts.
#[test]
fn a_reorg_that_drops_the_contract_creations_removes_the_accounts_and_refunds_the_deposits() {
    let w = World::new();
    let owner = key(ORACLE_OWNER_KEY);

    let create_oracle = create_oracle_tx(&owner, 4, w.validity_start_height);
    let oracle = create_oracle.recipient.clone();
    let create_bridge = create_bridge_tx(&owner, &oracle, w.validity_start_height);
    let bridge = create_bridge.recipient.clone();
    let funds_before = luna(&w.main, &funds_address());

    w.doomed_block(vec![create_oracle.clone()]);
    w.doomed_block(vec![create_bridge.clone()]);
    assert!(matches!(
        account(&w.main, &oracle),
        Some(Account::Oracle(_))
    ));
    assert!(matches!(
        account(&w.main, &bridge),
        Some(Account::Bridge(_))
    ));
    assert_eq!(
        luna(&w.main, &funds_address()),
        funds_before - ORACLE_DEPOSIT - BRIDGE_DEPOSIT
    );
    let oracle_created = account_bytes(&w.main, &oracle);
    let bridge_created = account_bytes(&w.main, &bridge);
    assert_same_chain(&[&w.main, &w.follower], "doomed branch");

    w.overtake(2);

    assert!(
        !matches!(account(&w.main, &oracle), Some(Account::Oracle(_))),
        "the oracle account must be gone"
    );
    assert!(
        !matches!(account(&w.main, &bridge), Some(Account::Bridge(_))),
        "the bridge account must be gone"
    );
    assert_eq!(luna(&w.main, &oracle), 0);
    assert_eq!(luna(&w.main, &bridge), 0);
    assert_eq!(
        luna(&w.main, &funds_address()),
        funds_before,
        "both deposits must be refunded"
    );

    w.canonical_block(vec![create_oracle]);
    w.canonical_block(vec![create_bridge]);
    assert_eq!(account_bytes(&w.main, &oracle), oracle_created);
    assert_eq!(account_bytes(&w.main, &bridge), bridge_created);
    assert_same_chain(&w.all(), "after re-creation");
}
