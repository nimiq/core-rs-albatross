//! Burn proofs against the oracle's ring buffer.
//!
//! Every oracle write replaces slot `i mod n` with `H(slot || value)`, a Merkle node over the
//! slot's previous value and the written one. A burn proof is therefore one `MerklePath`: from the
//! record's leaf through the record tree to the root the relayer wrote at index `i`, then through
//! the slot's buffer levels — the value the root overwrote on the left, then every value written to
//! the slot since on the right — ending in the slot's current value. Proofs never expire; they grow
//! by one node per rotation of the buffer.
//!
//! The oracle state here is built by real owner-signed updates, each committed, reverted and
//! re-applied by the test harness, and the expected buffer levels are computed from the written
//! values alone, independently of the contract.

use nimiq_account::{
    Account, BlockLogger, BlockState, BridgeContract, BurnProofError, OperationReceipt,
    OracleContract, TransactionLog,
};
use nimiq_hash::{Blake2bHash, Blake2bHasher, Hasher};
use nimiq_keys::{Address, KeyPair, PrivateKey};
use nimiq_primitives::{
    account::{AccountError, AccountType},
    coin::Coin,
    networks::NetworkId,
    policy::Policy,
    transaction::TransactionError,
};
use nimiq_serde::Serialize;
use nimiq_test_log::test;
use nimiq_test_utils::accounts_revert::TestCommitRevert;
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
use nimiq_utils::merkle::{MerklePath, MerkleProof};

const HASH_COUNT: u16 = 4;
const SOURCE_CHAIN_ID: u32 = 1;
const RELEASE_AMOUNT: u64 = 500;
const BRIDGE_DEPOSIT: u64 = 10_000;

fn oracle_addr() -> Address {
    Address::from([0x0Eu8; 20])
}

fn bridge_addr() -> Address {
    Address::from([0x0Bu8; 20])
}

fn oracle_owner() -> KeyPair {
    KeyPair::from(PrivateKey::from([0x11u8; 32]))
}

fn target_key() -> KeyPair {
    KeyPair::from(PrivateKey::from([0xAAu8; 32]))
}

fn target() -> Address {
    Address::from(&target_key().public)
}

fn blake2b(data: &[u8]) -> AnyHash {
    AnyHash::from(Blake2bHasher::default().digest(data))
}

fn zero() -> AnyHash {
    AnyHash::Blake2b(AnyHash32::default())
}

/// Reads `[0..20]` target address, `[20..28]` amount, `[28..36]` nonce, `[36..40]` burn block
/// height and `[40..44]` target chain ID, all little-endian.
fn bridge(max_proof_depth: u32) -> BridgeContract {
    let load = |offset, op, name: &str| {
        [
            ValidationOp::PushConst(offset),
            op,
            ValidationOp::Store(name.to_string()),
        ]
    };
    let program = [
        load(0, ValidationOp::LoadAddress, "target_address"),
        load(
            20,
            ValidationOp::LoadU64(Endianness::LittleEndian),
            "amount",
        ),
        load(
            28,
            ValidationOp::LoadU64(Endianness::LittleEndian),
            "target_nonce",
        ),
        load(
            36,
            ValidationOp::LoadU32(Endianness::LittleEndian),
            "burn_block_height",
        ),
        load(
            40,
            ValidationOp::LoadU32(Endianness::LittleEndian),
            "target_chain_id",
        ),
    ]
    .concat();

    BridgeContract {
        owner: Address::from([0x01u8; 20]),
        oracle_address: oracle_addr(),
        balance: Coin::from_u64_unchecked(BRIDGE_DEPOSIT),
        source_chain_id: SOURCE_CHAIN_ID,
        chain_config: ChainConfig {
            chain_id: SOURCE_CHAIN_ID,
            hash_function: zero(),
            address_format: AddressFormat::Nimiq,
            endianness: Endianness::LittleEndian,
            block_time: std::time::Duration::from_secs(60),
            validation_program: ValidationProgram::new(program),
            max_proof_depth,
        },
        transaction_count: 0,
    }
}

fn burn_record(nonce: u64) -> Vec<u8> {
    let mut record = target().as_bytes().to_vec();
    record.extend_from_slice(&RELEASE_AMOUNT.to_le_bytes());
    record.extend_from_slice(&nonce.to_le_bytes());
    record.extend_from_slice(&42u32.to_le_bytes());
    record.extend_from_slice(&SOURCE_CHAIN_ID.to_le_bytes());
    record
}

/// The record's leaf, exactly as consensus derives it from the record.
fn leaf(record: &[u8]) -> AnyHash {
    OutgoingTransaction {
        burn_transaction_data: record.to_vec(),
        merkle_proof: AnyMerkleProof::Blake2bPath(MerklePath::empty()),
        oracle_state_index: 0,
    }
    .extract_burn_transaction_hash(&zero())
    .unwrap()
}

/// A four-record tree, the relayer's commitment to a batch of burns.
struct RecordTree {
    records: Vec<Vec<u8>>,
    leaves: Vec<AnyHash>,
}

impl RecordTree {
    fn new(first_nonce: u64) -> Self {
        let records: Vec<Vec<u8>> = (first_nonce..first_nonce + 4).map(burn_record).collect();
        let leaves = records.iter().map(|record| leaf(record)).collect();
        RecordTree { records, leaves }
    }

    fn root(&self) -> AnyHash {
        let l = &self.leaves;
        l[0].digest(&l[1]).digest(&l[2].digest(&l[3]))
    }

    /// The tree levels of record `k`'s path as `(sibling, sibling_is_left)`, leaf first.
    fn levels(&self, k: usize) -> Vec<(AnyHash, bool)> {
        let l = &self.leaves;
        let other_pair = if k < 2 {
            l[2].digest(&l[3])
        } else {
            l[0].digest(&l[1])
        };
        vec![(l[k ^ 1].clone(), k % 2 == 1), (other_pair, k >= 2)]
    }
}

fn path(levels: &[(AnyHash, bool)]) -> AnyMerkleProof {
    let (siblings, left) = levels
        .iter()
        .map(|(hash, left)| {
            let bytes = <[u8; 32]>::try_from(hash.as_bytes()).unwrap();
            (Blake2bHash::from(bytes), *left)
        })
        .unzip();
    AnyMerkleProof::Blake2bPath(MerklePath::from_sibling_hashes(siblings, left))
}

fn proof(record: &[u8], levels: &[(AnyHash, bool)], index: u64) -> OutgoingTransaction {
    OutgoingTransaction {
        burn_transaction_data: record.to_vec(),
        merkle_proof: path(levels),
        oracle_state_index: index,
    }
}

/// An oracle written to only through owner-signed updates.
struct Ring {
    accounts: TestCommitRevert,
    oracle: OracleContract,
    /// Every value written so far; `written[i]` is the value written at global index `i`.
    written: Vec<AnyHash>,
}

impl Ring {
    fn new() -> Self {
        let oracle = OracleContract {
            owner: Address::from(&oracle_owner().public),
            balance: Coin::from_u64_unchecked(1_000),
            hash_count: HASH_COUNT,
            hashes: Vec::new(),
            latest_index: None,
        };
        let accounts = TestCommitRevert::with_initial_state(&[(
            oracle_addr(),
            Account::Oracle(oracle.clone()),
        )]);
        Ring {
            accounts,
            oracle,
            written: Vec::new(),
        }
    }

    /// Writes `values` in one owner-signed update. The harness commits it, reverts it, checks
    /// that the revert restored the oracle exactly, and applies it again.
    fn write(&mut self, values: Vec<AnyHash>) {
        let owner = oracle_owner();
        let data = IncomingOracleTransactionData::Update {
            hashes: values.clone(),
            proof: SignatureProof::default(),
        };
        let mut tx = Transaction::new_signaling(
            oracle_addr(),
            AccountType::Oracle,
            oracle_addr(),
            AccountType::Oracle,
            Coin::ZERO,
            data.serialize_to_vec(),
            1,
            NetworkId::UnitAlbatross,
        );
        let signature =
            SignatureProof::from_ed25519(owner.public, owner.sign(&tx.serialize_content()));
        tx.recipient_data =
            IncomingOracleTransactionData::set_signature_on_data(&tx.recipient_data, signature)
                .unwrap();
        self.accounts
            .test_commit_incoming_transaction(
                &mut self.oracle,
                &tx,
                &BlockState::new(1, 1, Policy::max_supported_version()),
                &mut TransactionLog::empty(),
                true,
            )
            .expect("the owner's update must commit");
        self.written.extend(values);
    }

    /// Writes `count` values that are not the root of any tree the tests prove against.
    fn write_others(&mut self, count: usize) {
        let values = (0..count)
            .map(|i| blake2b(format!("other root {}", self.written.len() + i).as_bytes()))
            .collect();
        self.write(values);
    }

    /// Writes a full rotation: every slot once.
    fn rotate(&mut self) {
        self.write_others(HASH_COUNT as usize);
    }

    /// Writes `root` and returns the index it was written at.
    fn publish(&mut self, root: AnyHash) -> u64 {
        self.write(vec![root]);
        self.written.len() as u64 - 1
    }

    /// The value slot `index mod n` held before `index` was written: the zero hash on its first
    /// use, else a node folded from every earlier value written to that slot.
    fn previous_slot_value(&self, index: u64) -> AnyHash {
        let n = HASH_COUNT as u64;
        (index % n..index)
            .step_by(n as usize)
            .fold(zero(), |slot, i| slot.digest(&self.written[i as usize]))
    }

    /// The buffer levels of a proof for a root written at `index`, against the current state: the
    /// value the root overwrote on the left, then every value written to the slot since on the
    /// right, one per rotation.
    fn buffer_levels(&self, index: u64) -> Vec<(AnyHash, bool)> {
        let n = HASH_COUNT as usize;
        let mut levels = vec![(self.previous_slot_value(index), true)];
        levels.extend(
            self.written
                .iter()
                .skip(index as usize + n)
                .step_by(n)
                .map(|value| (value.clone(), false)),
        );
        levels
    }

    fn verify(
        &self,
        bridge: &BridgeContract,
        proof: &OutgoingTransaction,
    ) -> Result<(), BurnProofError> {
        bridge.verify_burn_proof(&self.oracle, proof)
    }
}

fn concat(tree: Vec<(AnyHash, bool)>, buffer: &[(AnyHash, bool)]) -> Vec<(AnyHash, bool)> {
    tree.into_iter().chain(buffer.iter().cloned()).collect()
}

/// A root written to a slot for the first time: each record proves through the tree and then the
/// slot's zero hash. The tree path alone, which a single-entry chain to the previous index used to
/// accept, ends in the root rather than the slot's value and is refused, and so is a buffer node on
/// the wrong side.
#[test]
fn a_proof_runs_from_the_leaf_through_the_tree_and_the_slot() {
    let mut ring = Ring::new();
    ring.write_others(1);
    let tree = RecordTree::new(1);
    let index = ring.publish(tree.root());
    assert_eq!(index, 1);
    let bridge = bridge(64);

    for k in 0..4 {
        let levels = concat(tree.levels(k), &[(zero(), true)]);
        assert!(
            ring.verify(&bridge, &proof(&tree.records[k], &levels, index))
                .is_ok(),
            "record {k} must prove through the tree and the slot",
        );
    }

    assert!(matches!(
        ring.verify(&bridge, &proof(&tree.records[0], &tree.levels(0), index)),
        Err(BurnProofError::SlotMismatch(1))
    ));
    assert!(matches!(
        ring.verify(
            &bridge,
            &proof(
                &tree.records[0],
                &concat(tree.levels(0), &[(zero(), false)]),
                index
            )
        ),
        Err(BurnProofError::SlotMismatch(1))
    ));
    // A record that is not in the tree.
    assert!(matches!(
        ring.verify(
            &bridge,
            &proof(
                &burn_record(9),
                &concat(tree.levels(0), &[(zero(), true)]),
                index
            )
        ),
        Err(BurnProofError::SlotMismatch(1))
    ));
}

/// A proof built when its root landed keeps working through any number of rotations once it is
/// extended by the value written to its slot in each of them, and the path grows by exactly one
/// node per rotation. Checked for a root in a slot's first use and for one that overwrote an
/// earlier entry, and with writes that leave the slot alone in between.
#[test]
fn a_proof_stays_valid_after_k_rotations_with_k_extra_nodes() {
    for others_before in [1, HASH_COUNT as usize + 2] {
        let mut ring = Ring::new();
        ring.write_others(others_before.min(HASH_COUNT as usize));
        if others_before > HASH_COUNT as usize {
            ring.write_others(others_before - HASH_COUNT as usize);
        }
        let tree = RecordTree::new(1);
        let index = ring.publish(tree.root());
        let first_buffer_node = ring.previous_slot_value(index);
        assert_eq!(
            first_buffer_node == zero(),
            index < HASH_COUNT as u64,
            "index {index}: only a slot's first use overwrites the zero hash",
        );

        let original = concat(tree.levels(2), &ring.buffer_levels(index));
        assert_eq!(original.len(), 3);

        for k in 0..=3usize {
            if k > 0 {
                // A partial batch that leaves the slot alone, then the rest of the rotation.
                ring.write_others(HASH_COUNT as usize - 1);
                assert!(ring
                    .verify(
                        &bridge(64),
                        &proof(
                            &tree.records[2],
                            &concat(tree.levels(2), &ring.buffer_levels(index)),
                            index
                        )
                    )
                    .is_ok());
                ring.write_others(1);
            }

            let levels = concat(tree.levels(2), &ring.buffer_levels(index));
            assert_eq!(
                levels.len(),
                original.len() + k,
                "after {k} rotations the path carries {k} extra nodes"
            );
            assert_eq!(
                levels[..original.len()],
                original[..],
                "the proof only grows"
            );
            assert!(
                ring.verify(&bridge(64), &proof(&tree.records[2], &levels, index))
                    .is_ok(),
                "index {index} after {k} rotations",
            );
            // The bound counts tree and buffer levels together.
            assert!(ring
                .verify(
                    &bridge(levels.len() as u32),
                    &proof(&tree.records[2], &levels, index)
                )
                .is_ok());
            assert!(matches!(
                ring.verify(
                    &bridge(levels.len() as u32 - 1),
                    &proof(&tree.records[2], &levels, index)
                ),
                Err(BurnProofError::TooDeep { depth, max_depth })
                    if depth == levels.len() && max_depth == levels.len() as u32 - 1
            ));
        }

        assert!(
            ring.oracle.earliest_index().unwrap() > index,
            "the index has long left the window but still names its slot"
        );
    }
}

/// Once a slot is written again, a proof that stops at the slot's earlier value — however it got
/// there — is refused. So is a proof extended by a value that was never written to the slot, one
/// that names an index in another slot, and one for an index not written yet.
#[test]
fn a_proof_that_ends_in_a_superseded_slot_value_is_rejected() {
    let mut ring = Ring::new();
    let tree = RecordTree::new(1);
    let index = ring.publish(tree.root());
    let original = concat(tree.levels(1), &ring.buffer_levels(index));
    let bridge = bridge(64);
    assert!(ring
        .verify(&bridge, &proof(&tree.records[1], &original, index))
        .is_ok());

    ring.rotate();
    ring.rotate();

    // Ends in the slot's value before both rotations, and before the second.
    for rotations in 0..2 {
        let stale = concat(tree.levels(1), &ring.buffer_levels(index)[..1 + rotations]);
        assert!(matches!(
            ring.verify(&bridge, &proof(&tree.records[1], &stale, index)),
            Err(BurnProofError::SlotMismatch(0))
        ));
    }

    let current = concat(tree.levels(1), &ring.buffer_levels(index));
    assert!(ring
        .verify(&bridge, &proof(&tree.records[1], &current, index))
        .is_ok());

    // A buffer node that was never written to the slot.
    let mut forged = current.clone();
    let last = forged.len() - 1;
    forged[last] = (blake2b(b"never written"), false);
    assert!(matches!(
        ring.verify(&bridge, &proof(&tree.records[1], &forged, index)),
        Err(BurnProofError::SlotMismatch(0))
    ));

    // The right path, but an index naming another slot.
    assert!(matches!(
        ring.verify(&bridge, &proof(&tree.records[1], &current, index + 1)),
        Err(BurnProofError::SlotMismatch(1))
    ));

    // An index not written yet.
    let unwritten = ring.oracle.latest_index.unwrap() + 1;
    assert!(matches!(
        ring.verify(&bridge, &proof(&tree.records[1], &current, unwritten)),
        Err(BurnProofError::UnwrittenIndex(i)) if i == unwritten
    ));
}

/// The proof is one Merkle path; the stack-based multi-proof format is refused even where it folds
/// to the right value.
#[test]
fn only_merkle_paths_are_accepted() {
    let mut ring = Ring::new();
    let record = burn_record(1);
    let index = ring.publish(leaf(&record));
    let AnyHash::Blake2b(leaf_bytes) = leaf(&record) else {
        unreachable!()
    };
    let leaf_hash = Blake2bHash::from(leaf_bytes.0);

    // The slot's value is H(zero || leaf), a two-leaf tree with the zero hash first.
    let multi_proof = MerkleProof::<Blake2bHash>::new(
        &[Blake2bHash::default(), leaf_hash.clone()],
        std::slice::from_ref(&leaf_hash),
    );
    assert_eq!(
        AnyHash::from(multi_proof.compute_root(vec![leaf_hash]).unwrap()),
        ring.oracle.hashes[0],
    );
    let stack_proof = OutgoingTransaction {
        burn_transaction_data: record.clone(),
        merkle_proof: AnyMerkleProof::Blake2b(multi_proof),
        oracle_state_index: index,
    };
    assert!(matches!(
        ring.verify(&bridge(64), &stack_proof),
        Err(BurnProofError::NotAMerklePath)
    ));
    assert!(ring
        .verify(&bridge(64), &proof(&record, &[(zero(), true)], index))
        .is_ok());
}

/// The full release path after the oracle rotated past the burn's root: the original proof is
/// refused without moving funds, and the proof extended by one node per rotation pays out the
/// burn, with a revert that restores the bridge exactly.
#[test]
fn a_release_commits_after_rotations_and_reverts_exactly() {
    let mut ring = Ring::new();
    let tree = RecordTree::new(1);
    let index = ring.publish(tree.root());
    let original = concat(tree.levels(0), &ring.buffer_levels(index));
    ring.rotate();
    ring.write_others(2);
    ring.rotate();
    let extended = concat(tree.levels(0), &ring.buffer_levels(index));
    assert_eq!(extended.len(), original.len() + 2);

    let accounts = TestCommitRevert::with_initial_state(&[
        (oracle_addr(), Account::Oracle(ring.oracle.clone())),
        (bridge_addr(), Account::Bridge(bridge(64))),
    ]);
    let block_state = BlockState::new(1, 1, Policy::max_supported_version());

    let release = |levels: &[(AnyHash, bool)]| {
        let mut data = OutgoingBridgeTransactionData {
            burn_proof: proof(&tree.records[0], levels, index),
            proof: SignatureProof::default(),
        };
        let mut tx = Transaction::new_extended(
            bridge_addr(),
            AccountType::Bridge,
            data.serialize_to_vec(),
            target(),
            AccountType::Basic,
            vec![],
            Coin::from_u64_unchecked(RELEASE_AMOUNT),
            Coin::ZERO,
            1,
            NetworkId::UnitAlbatross,
        );
        let key = target_key();
        data.set_signature(SignatureProof::from_ed25519(
            key.public,
            key.sign(&tx.serialize_content()),
        ));
        tx.sender_data = data.serialize_to_vec();
        tx
    };

    let mut bridge_account = bridge(64);
    assert_eq!(
        accounts.test_commit_outgoing_transaction(
            &mut bridge_account,
            &release(&original),
            &block_state,
            &mut TransactionLog::empty(),
            false,
        ),
        Err(AccountError::InvalidTransaction(
            TransactionError::InvalidProof
        )),
    );
    assert_eq!(bridge_account, bridge(64));

    let receipts = accounts
        .commit_and_test(
            &[release(&extended)],
            &[],
            &block_state,
            &mut BlockLogger::empty(),
        )
        .expect("the extended release must commit");
    assert!(matches!(receipts.transactions[0], OperationReceipt::Ok(_)));
    assert_eq!(
        accounts.get_complete(&bridge_addr(), None).balance(),
        Coin::from_u64_unchecked(BRIDGE_DEPOSIT - RELEASE_AMOUNT),
    );
    assert_eq!(
        accounts.get_complete(&target(), None).balance(),
        Coin::from_u64_unchecked(RELEASE_AMOUNT),
    );
}
