//! Bridge and oracle transactions for tests against a real chain: contract creations, the
//! relayer's oracle updates, locks, and the releases that targets sign.
//!
//! The bridge reads its burn records with a little-endian fixed-offset program (see
//! [`chain_config`]). Keys are fixed rather than generated, so every run builds the same
//! transactions.

use nimiq_block::Block;
use nimiq_blockchain_interface::{AbstractBlockchain, PushResult};
use nimiq_genesis::NetworkId;
use nimiq_hash::{Blake2bHash, Blake2bHasher, HashOutput, Hasher};
use nimiq_keys::{Address, KeyPair, PrivateKey};
use nimiq_primitives::{account::AccountType, coin::Coin, policy::upgrades};
use nimiq_serde::{Deserialize, Serialize};
use nimiq_transaction::{
    account::{
        bridge_contract::OutgoingBridgeTransactionData,
        htlc_contract::{AnyHash, AnyHash32, HashType},
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

use crate::{block_production::TemporaryBlockProducer, blockchain::REWARD_KEY};

pub const SOURCE_CHAIN_ID: u32 = 1;
pub const ORACLE_DEPOSIT: u64 = 1_000;
pub const BRIDGE_DEPOSIT: u64 = 10_000;
pub const BURN_BLOCK_HEIGHT: u32 = 42;
/// The contract on the source chain whose records the bridge releases.
pub const SOURCE_CONTRACT: [u8; 20] = [0x5Cu8; 20];

/// The oracle owner. Fixed rather than generated so every run builds the same transactions.
pub const ORACLE_OWNER_KEY: &str =
    "9d5bd02379e7e45cf515c788048f5cf3c454ffabd3e83bd1d7667716c325c3c0";
/// The burn target, which signs its release. The fee comes out of the burned amount, so the key
/// needs no funds.
pub const TARGET_KEY: &str = "0f0e0d0c0b0a09080706050403020100ffeeddccbbaa99887766554433221100";

pub fn key(hex_key: &str) -> KeyPair {
    KeyPair::from(PrivateKey::deserialize_from_vec(&hex::decode(hex_key).unwrap()).unwrap())
}

/// The genesis account that funds everything: contract deposits, locks and the relayer's updates.
pub fn funds() -> KeyPair {
    key(REWARD_KEY)
}

pub fn funds_address() -> Address {
    Address::from(&funds().public)
}

/// Little-endian fixed-offset program: target_address [0..20], amount [20..28],
/// target_nonce [28..36], burn_block_height [36..40], target_chain_id [40..44],
/// source_contract [44..64], bridge_address [64..84].
pub fn chain_config() -> ChainConfig {
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
            ValidationOp::PushConst(44),
            ValidationOp::LoadAddress,
            ValidationOp::Store("source_contract".to_string()),
            ValidationOp::PushConst(64),
            ValidationOp::LoadAddress,
            ValidationOp::Store("bridge_address".to_string()),
        ]),
        max_proof_depth: 64,
        source_contract: SOURCE_CONTRACT.to_vec(),
    }
}

/// A burn record from `SOURCE_CONTRACT`, releasable by the bridge at `bridge`.
pub fn burn_data(bridge: &Address, target: &Address, amount: u64, nonce: u64) -> Vec<u8> {
    let mut data = Vec::with_capacity(84);
    data.extend_from_slice(target.as_bytes());
    data.extend_from_slice(&amount.to_le_bytes());
    data.extend_from_slice(&nonce.to_le_bytes());
    data.extend_from_slice(&BURN_BLOCK_HEIGHT.to_le_bytes());
    data.extend_from_slice(&SOURCE_CHAIN_ID.to_le_bytes());
    data.extend_from_slice(&SOURCE_CONTRACT);
    data.extend_from_slice(bridge.as_bytes());
    data
}

pub fn blake2b(data: &[u8]) -> AnyHash {
    let hash = Blake2bHasher::default().digest(data);
    AnyHash::Blake2b(AnyHash32::from(
        <[u8; 32]>::try_from(hash.as_bytes()).unwrap(),
    ))
}

/// The record-tree leaf of a burn record, `blake2b(blake2b(record))`.
pub fn record_leaf(record: &[u8]) -> AnyHash {
    blake2b(blake2b(record).as_bytes())
}

pub fn create_oracle_tx(
    owner: &KeyPair,
    hash_count: u16,
    validity_start_height: u32,
) -> Transaction {
    TransactionBuilder::new_create_oracle(
        &funds(),
        Address::from(&owner.public),
        hash_count,
        HashType::Blake2b,
        Coin::from_u64_unchecked(ORACLE_DEPOSIT),
        Coin::ZERO,
        validity_start_height,
        NetworkId::UnitAlbatross,
    )
    .expect("a valid oracle creation")
}

pub fn create_bridge_tx(
    owner: &KeyPair,
    oracle: &Address,
    validity_start_height: u32,
) -> Transaction {
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
pub fn oracle_update_tx(
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
pub fn lock_tx(bridge: &Address, amount: u64, validity_start_height: u32) -> Transaction {
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

/// A Blake2b Merkle path from `(sibling, sibling_is_left)` nodes, leaf first.
pub fn blake2b_path(nodes: &[(AnyHash, bool)]) -> MerklePath<Blake2bHash> {
    let (siblings, left) = nodes
        .iter()
        .map(|(hash, left)| {
            let bytes = <[u8; 32]>::try_from(hash.as_bytes()).unwrap();
            (Blake2bHash::from(bytes), *left)
        })
        .unzip();
    MerklePath::from_sibling_hashes(siblings, left)
}

/// The proof of a burn whose single-leaf tree root was written to an oracle slot for the first
/// time, extended by the values written to that slot since: the slot's zero hash on the left, then
/// one right sibling per rotation.
pub fn burn_proof(
    burn: Vec<u8>,
    oracle_state_index: u64,
    rotations: &[AnyHash],
) -> OutgoingTransaction {
    let zero = AnyHash::Blake2b(AnyHash32::default());
    let nodes: Vec<(AnyHash, bool)> = std::iter::once((zero, true))
        .chain(rotations.iter().map(|value| (value.clone(), false)))
        .collect();
    OutgoingTransaction {
        burn_transaction_data: burn,
        merkle_proof: AnyMerkleProof::Blake2bPath(blake2b_path(&nodes)),
        oracle_state_index,
    }
}

/// A release carrying `burn_proof`, signed by `signer`, without a fee.
pub fn release_tx(
    bridge: &Address,
    target: &Address,
    amount: u64,
    burn_proof: OutgoingTransaction,
    signer: &KeyPair,
    validity_start_height: u32,
) -> Transaction {
    release_tx_with_fee(
        bridge,
        target,
        amount,
        0,
        burn_proof,
        signer,
        validity_start_height,
    )
}

/// A release paying `value` to `target` and `fee` to the block reward, carrying `burn_proof` and
/// signed by `signer`. It is valid only if `value + fee` is the burned amount.
pub fn release_tx_with_fee(
    bridge: &Address,
    target: &Address,
    value: u64,
    fee: u64,
    burn_proof: OutgoingTransaction,
    signer: &KeyPair,
    validity_start_height: u32,
) -> Transaction {
    let mut bridge_data = OutgoingBridgeTransactionData {
        burn_proof,
        proof: SignatureProof::default(),
    };
    let mut tx = Transaction::new_extended(
        bridge.clone(),
        AccountType::Bridge,
        bridge_data.serialize_to_vec(),
        target.clone(),
        AccountType::Basic,
        vec![],
        Coin::from_u64_unchecked(value),
        Coin::from_u64_unchecked(fee),
        validity_start_height,
        NetworkId::UnitAlbatross,
    );
    bridge_data.set_signature(SignatureProof::from_ed25519(
        signer.public,
        signer.sign(&tx.serialize_content()),
    ));
    tx.sender_data = bridge_data.serialize_to_vec();
    tx
}

/// A chain on protocol v3 that has an oracle owned by [`ORACLE_OWNER_KEY`] and a bridge on top of
/// it, both funded from [`funds`].
pub struct BridgeChain {
    pub node: TemporaryBlockProducer,
    pub owner: KeyPair,
    pub oracle: Address,
    pub bridge: Address,
    /// Validity start height shared by the transactions this chain builds, dated ahead so that
    /// they stay valid for the whole test.
    pub validity_start_height: u32,
}

impl BridgeChain {
    /// Creates the oracle, with `hash_count` slots, and then the bridge, one block each.
    pub fn new(hash_count: u16) -> Self {
        let node = TemporaryBlockProducer::new_with_protocol_version(
            upgrades::v3::BRIDGE_ORACLE_CONTRACTS,
        );
        let validity_start_height = node.blockchain.read().block_number() + 16;
        let owner = key(ORACLE_OWNER_KEY);

        let create_oracle = create_oracle_tx(&owner, hash_count, validity_start_height);
        let oracle = create_oracle.recipient.clone();
        let create_bridge = create_bridge_tx(&owner, &oracle, validity_start_height);
        let bridge = create_bridge.recipient.clone();

        let chain = BridgeChain {
            node,
            owner,
            oracle,
            bridge,
            validity_start_height,
        };
        chain.push_block(vec![create_oracle]);
        chain.push_block(vec![create_bridge]);
        chain
    }

    /// A burn of `amount` luna to `target` with `nonce`, releasable by this chain's bridge.
    pub fn burn(&self, target: &Address, amount: u64, nonce: u64) -> Vec<u8> {
        burn_data(&self.bridge, target, amount, nonce)
    }

    /// The relayer's update writing `hashes` to the oracle.
    pub fn update(&self, hashes: Vec<AnyHash>) -> Transaction {
        oracle_update_tx(
            &self.oracle,
            &self.owner,
            hashes,
            self.validity_start_height,
        )
    }

    /// A release of `burn`, of `amount` luna to the address of `target`, proven against oracle
    /// index `oracle_state_index` as written for the first time and rotated by `rotations` since,
    /// with the whole amount paid out as value.
    pub fn release(
        &self,
        target: &KeyPair,
        amount: u64,
        burn: Vec<u8>,
        oracle_state_index: u64,
        rotations: &[AnyHash],
    ) -> Transaction {
        self.release_with_fee(target, amount, 0, burn, oracle_state_index, rotations)
    }

    /// Like [`Self::release`], with `fee` of the burned `amount` going to the block reward.
    pub fn release_with_fee(
        &self,
        target: &KeyPair,
        amount: u64,
        fee: u64,
        burn: Vec<u8>,
        oracle_state_index: u64,
        rotations: &[AnyHash],
    ) -> Transaction {
        release_tx_with_fee(
            &self.bridge,
            &Address::from(&target.public),
            amount - fee,
            fee,
            burn_proof(burn, oracle_state_index, rotations),
            target,
            self.validity_start_height,
        )
    }

    /// A user locking `amount` luna into the bridge.
    pub fn lock(&self, amount: u64) -> Transaction {
        lock_tx(&self.bridge, amount, self.validity_start_height)
    }

    /// Produces the next micro block from `transactions`, as a validator does, without applying it.
    pub fn produce_block(&self, transactions: Vec<Transaction>) -> Block {
        let block = self
            .node
            .next_block_no_push_with_txs(vec![], false, transactions);
        assert!(block.is_micro(), "the test must not cross a macro block");
        block
    }

    /// Produces the next micro block from `transactions` and applies it.
    pub fn push_block(&self, transactions: Vec<Transaction>) -> Block {
        let block = self.produce_block(transactions);
        assert_eq!(self.node.push(block.clone()), Ok(PushResult::Extended));
        block
    }

    /// The balance of `address`, in luna.
    pub fn luna(&self, address: &Address) -> u64 {
        self.node
            .blockchain
            .read()
            .get_account_if_complete(address)
            .map_or(0, |account| u64::from(account.balance()))
    }
}
