use std::{sync::Arc, time::Duration};

use nimiq_account::{Account, BridgeContract};
use nimiq_block::{Block, MicroBlock, MicroBody, MicroHeader};
use nimiq_blockchain::{Blockchain, BlockchainConfig};
use nimiq_bls::KeyPair as BlsKeyPair;
use nimiq_database::{mdbx::MdbxDatabase, traits::WriteTransaction};
use nimiq_genesis_builder::GenesisBuilder;
use nimiq_hash::{Blake2bHash, Blake2sHash, Hash};
use nimiq_keys::{
    Address, Ed25519PublicKey as SchnorrPublicKey, KeyPair as SchnorrKeyPair, SecureGenerate,
};
use nimiq_mempool::{config::MempoolConfig, mempool::Mempool, verify::VerifyErr};
use nimiq_primitives::{
    account::AccountType, coin::Coin, key_nibbles::KeyNibbles, networks::NetworkId, policy::Policy,
};
use nimiq_serde::Serialize;
use nimiq_test_log::test;
use nimiq_test_utils::test_rng;
use nimiq_transaction::{
    account::{
        bridge_contract::OutgoingBridgeTransactionData,
        htlc_contract::{AnyHash, AnyHash32},
    },
    bridge_contract::{
        AddressFormat, AnyMerkleProof, ChainConfig, Endianness, OutgoingTransaction,
        ValidationProgram,
    },
    SignatureProof, Transaction,
};
use nimiq_utils::{merkle::MerkleProof, time::OffsetTime};
use nimiq_vrf::VrfSeed;
use parking_lot::RwLock;

// These integration tests exercise the signer-pays fee model for outgoing bridge
// burn-releases end-to-end through the real `Mempool`. Outgoing bridge releases are
// permissionless and pay their fee from the burn-proof *signer* (not the bridge), so
// the mempool must reserve the fee against the signer's account at admission
// (`Blockchain::reserve_bridge_signer_fee`), release it on removal, and re-reserve it
// on the per-block rebuild. Mempool admission only verifies the self-signature — it
// does not validate the burn proof or oracle state — so a self-signed release with a
// placeholder proof is enough to drive these paths.

const SOURCE_CHAIN_ID: u32 = 1;
const BRIDGE_BALANCE: u64 = 1_000_000;
const RELEASE_VALUE: u64 = 100;
const FEE: u64 = 10;

fn bridge_address() -> Address {
    Address::from([0x0B; 20])
}

fn target_address() -> Address {
    Address::from([0xAA; 20])
}

fn bridge_chain_config() -> ChainConfig {
    ChainConfig {
        chain_id: SOURCE_CHAIN_ID,
        hash_function: AnyHash::Blake2b(AnyHash32::default()),
        address_format: AddressFormat::Nimiq,
        endianness: Endianness::LittleEndian,
        block_time: Duration::from_secs(60),
        validation_program: ValidationProgram::empty(),
        max_proof_depth: 64,
    }
}

struct BridgeFixture {
    blockchain: Arc<RwLock<Blockchain>>,
    mempool: Mempool,
    signer: SchnorrKeyPair,
    validity_start_height: u32,
}

/// Builds a blockchain whose genesis funds `signer_balance` to the burn-proof signer,
/// seeds a `BridgeContract` (balance `BRIDGE_BALANCE`) directly into the accounts tree,
/// and returns a fresh mempool over it.
fn setup(signer_balance: u64) -> BridgeFixture {
    let mut rng = test_rng(true);

    let signer = SchnorrKeyPair::generate(&mut rng);
    let signer_address = Address::from(&signer.public);

    let mut genesis_builder = GenesisBuilder::default();
    genesis_builder.with_network(NetworkId::UnitAlbatross);
    genesis_builder.with_basic_account(
        signer_address.clone(),
        Coin::from_u64_unchecked(signer_balance),
    );
    // Genesis requires at least one validator.
    genesis_builder.with_genesis_validator(
        Address::from(&SchnorrKeyPair::generate(&mut rng)),
        SchnorrPublicKey::from([0u8; 32]),
        BlsKeyPair::generate(&mut rng).public_key,
        Address::default(),
        None,
        None,
        false,
    );

    let env = MdbxDatabase::new_volatile(Default::default()).unwrap();
    let genesis_info = genesis_builder.generate(env.clone()).unwrap();

    let genesis_block = match genesis_info.block {
        Block::Macro(mut block) => {
            block.header.block_number = Policy::genesis_block_number();
            // Bridge/oracle contracts activate at protocol version 3
            // (`upgrades::v3::BRIDGE_ORACLE_CONTRACTS`). `blockchain.protocol_version()`
            // reads the head block's version, so the genesis must report a
            // bridge-enabled version or the mempool rejects every bridge transaction.
            block.header.version = Policy::max_supported_version();
            Block::Macro(block)
        }
        Block::Micro(_) => panic!("expected a macro genesis block"),
    };

    let time = Arc::new(OffsetTime::new());
    let blockchain = Arc::new(RwLock::new(
        Blockchain::with_genesis(
            env,
            BlockchainConfig::default(),
            time,
            NetworkId::UnitAlbatross,
            genesis_block,
            genesis_info.accounts,
        )
        .unwrap(),
    ));

    seed_bridge_account(&blockchain, &signer_address);

    let validity_start_height = 1 + Policy::genesis_block_number();
    let mempool = Mempool::new(Arc::clone(&blockchain), MempoolConfig::default());

    BridgeFixture {
        blockchain,
        mempool,
        signer,
        validity_start_height,
    }
}

/// Inserts a `BridgeContract` into the live accounts tree. The mempool reads accounts
/// directly from the tree (it does not re-verify them against the head's state root),
/// so a direct insert is sufficient to make the bridge sender resolvable — and avoids
/// the oracle + valid-proof scaffolding a full contract-creation transaction would need.
fn seed_bridge_account(blockchain: &Arc<RwLock<Blockchain>>, owner: &Address) {
    let bridge = BridgeContract {
        owner: owner.clone(),
        oracle_address: Address::from([0x0E; 20]),
        balance: Coin::from_u64_unchecked(BRIDGE_BALANCE),
        source_chain_id: SOURCE_CHAIN_ID,
        chain_config: bridge_chain_config(),
        transaction_count: 0,
    };

    let bc = blockchain.read();
    let mut raw_txn = bc.write_transaction();
    let mut txn: nimiq_trie::WriteTransactionProxy = (&mut raw_txn).into();
    let tree = &bc.state().accounts.tree;
    tree.put(
        &mut txn,
        &KeyNibbles::from(&bridge_address()),
        Account::Bridge(bridge),
    )
    .expect("failed to seed bridge account");
    tree.update_root(&mut txn).expect("failed to update root");
    drop(txn);
    raw_txn.commit();
}

/// Builds a self-signed outgoing bridge burn-release. `value` distinguishes otherwise
/// identical releases so they hash differently. The burn proof is a placeholder — the
/// mempool never validates it, only the sender_data self-signature.
fn build_release(fixture: &BridgeFixture, value: u64, fee: u64) -> Transaction {
    let outgoing = OutgoingTransaction {
        burn_transaction_data: vec![0u8; 32],
        merkle_proof: AnyMerkleProof::Blake2b(MerkleProof::<Blake2bHash>::new(&[], &[])),
        oracle_state_index: 0,
    };
    let mut bridge_data = OutgoingBridgeTransactionData {
        burn_proof: outgoing,
        proof: SignatureProof::default(),
    };

    let mut tx = Transaction::new_extended(
        bridge_address(),
        AccountType::Bridge,
        bridge_data.serialize_to_vec(),
        target_address(),
        AccountType::Basic,
        vec![],
        Coin::from_u64_unchecked(value),
        Coin::from_u64_unchecked(fee),
        fixture.validity_start_height,
        NetworkId::UnitAlbatross,
    );

    // Sign the content while sender_data carries the default (empty) proof, then embed
    // the real proof — the signature is verified over the default-proof content.
    let signature = fixture.signer.sign(&tx.serialize_content());
    bridge_data.set_signature(SignatureProof::from_ed25519(
        fixture.signer.public,
        signature,
    ));
    tx.sender_data = bridge_data.serialize_to_vec();

    tx
}

/// A dummy micro block carrying `transactions`, used to drive `Mempool::update`.
fn dummy_micro_block(transactions: Vec<Transaction>) -> Block {
    let header = MicroHeader {
        network: NetworkId::UnitAlbatross,
        version: 0,
        block_number: 1 + Policy::genesis_block_number(),
        timestamp: 0,
        parent_hash: Blake2bHash::default(),
        seed: VrfSeed::default(),
        extra_data: vec![0; 1],
        state_root: Blake2bHash::default(),
        body_root: Blake2sHash::default(),
        diff_root: Blake2bHash::default(),
        history_root: Blake2bHash::default(),
        ..Default::default()
    };
    let body = MicroBody {
        equivocation_proofs: vec![],
        transactions: transactions
            .into_iter()
            .map(nimiq_transaction::ExecutedTransaction::Ok)
            .collect(),
    };
    Block::Micro(MicroBlock {
        header,
        body: Some(body),
        justification: None,
    })
}

/// The signer fee is reserved against the signer at admission: a signer funded for only
/// one fee gets exactly one of two burn-releases admitted, and the second is rejected on
/// the signer's balance (not the bridge's, which has ample funds).
#[test]
fn signer_fee_is_reserved_against_signer_on_admission() {
    let fixture = setup(FEE);

    let tx1 = build_release(&fixture, RELEASE_VALUE, FEE);
    let tx2 = build_release(&fixture, RELEASE_VALUE + 1, FEE);

    assert!(
        fixture.mempool.add_transaction(tx1.clone(), None).is_ok(),
        "first burn-release should be admitted"
    );
    let res2 = fixture.mempool.add_transaction(tx2.clone(), None);
    assert!(
        matches!(res2, Err(VerifyErr::InvalidAccount(_))),
        "second burn-release should be rejected because the signer cannot cover a second fee, got {res2:?}"
    );

    assert_eq!(fixture.mempool.num_transactions(), 1);
    assert!(fixture.mempool.contains_transaction_by_hash(&tx1.hash()));
    assert!(!fixture.mempool.contains_transaction_by_hash(&tx2.hash()));
}

/// Control for the test above: when the signer can cover both fees, both burn-releases
/// are admitted. This isolates the signer-fee reservation as the cause of the rejection.
#[test]
fn both_releases_admitted_when_signer_covers_all_fees() {
    let fixture = setup(2 * FEE);

    let tx1 = build_release(&fixture, RELEASE_VALUE, FEE);
    let tx2 = build_release(&fixture, RELEASE_VALUE + 1, FEE);

    assert!(fixture.mempool.add_transaction(tx1, None).is_ok());
    assert!(fixture.mempool.add_transaction(tx2, None).is_ok());
    assert_eq!(fixture.mempool.num_transactions(), 2);
}

/// The signer fee is released when the release leaves the mempool: after the admitted
/// release is removed (mined), the signer's reserved fee is freed and the previously
/// rejected release can be admitted.
#[test]
fn signer_fee_is_released_when_release_is_removed() {
    let fixture = setup(FEE);

    let tx1 = build_release(&fixture, RELEASE_VALUE, FEE);
    let tx2 = build_release(&fixture, RELEASE_VALUE + 1, FEE);

    assert!(fixture.mempool.add_transaction(tx1.clone(), None).is_ok());
    assert!(
        fixture.mempool.add_transaction(tx2.clone(), None).is_err(),
        "second release must be rejected while the signer fee is reserved"
    );

    // Mine tx1: `update` sees it as known and removes it, releasing the signer fee.
    let block = dummy_micro_block(vec![tx1.clone()]);
    fixture.mempool.update(&[(block.hash(), block)], &[]);
    assert!(!fixture.mempool.contains_transaction_by_hash(&tx1.hash()));

    // With the fee freed, tx2 is now admissible.
    assert!(
        fixture.mempool.add_transaction(tx2.clone(), None).is_ok(),
        "second release should be admitted after the first one's fee is released"
    );
    assert_eq!(fixture.mempool.num_transactions(), 1);
    assert!(fixture.mempool.contains_transaction_by_hash(&tx2.hash()));
}

/// The signer fee is re-reserved on the per-block rebuild (`recompute_sender_balances`):
/// after an unrelated block adoption forces a rebuild, the admitted release survives and
/// the signer's fee remains reserved, so a further release is still rejected.
#[test]
fn signer_fee_is_re_reserved_on_rebuild() {
    let fixture = setup(FEE);

    let tx1 = build_release(&fixture, RELEASE_VALUE, FEE);
    assert!(fixture.mempool.add_transaction(tx1.clone(), None).is_ok());

    // A non-empty adopted set forces `recompute_sender_balances` over every sender,
    // re-reserving tx1's value and signer fee from scratch.
    let block = dummy_micro_block(vec![]);
    fixture.mempool.update(&[(block.hash(), block)], &[]);

    assert_eq!(fixture.mempool.num_transactions(), 1);
    assert!(
        fixture.mempool.contains_transaction_by_hash(&tx1.hash()),
        "the valid release must survive the rebuild"
    );

    // The fee is still reserved against the signer after the rebuild.
    let tx2 = build_release(&fixture, RELEASE_VALUE + 1, FEE);
    assert!(
        fixture.mempool.add_transaction(tx2, None).is_err(),
        "signer fee must remain reserved after the rebuild"
    );
}
