//! Commits bridge and oracle transactions built by `TransactionBuilder`, the path the RPC server
//! uses, so that the builders are checked against the accounts tree and not only against
//! `Transaction::verify`.

use nimiq_account::{
    Account, BasicAccount, BlockLogger, BlockState, BridgeContract, OperationReceipt,
    OracleContract,
};
use nimiq_hash::{Blake2bHasher, HashOutput, Hasher};
use nimiq_keys::{Address, KeyPair};
use nimiq_primitives::{coin::Coin, networks::NetworkId, policy::upgrades};
use nimiq_test_log::test;
use nimiq_test_utils::accounts_revert::TestCommitRevert;
use nimiq_transaction::{
    account::htlc_contract::{AnyHash, AnyHash32},
    bridge_contract::{
        AddressFormat, AnyMerkleProof, ChainConfig, Endianness, OutgoingTransaction, ValidationOp,
        ValidationProgram,
    },
    Transaction,
};
use nimiq_transaction_builder::TransactionBuilder;
use nimiq_utils::{key_rng::SecureGenerate, merkle::MerklePath};

const NETWORK_ID: NetworkId = NetworkId::UnitAlbatross;
const PROTOCOL_VERSION: u16 = upgrades::v3::BRIDGE_ORACLE_CONTRACTS;

const SOURCE_CHAIN_ID: u32 = 1;
const BURN_BLOCK_HEIGHT: u32 = 42;

fn coin(value: u64) -> Coin {
    Coin::from_u64_unchecked(value)
}

fn basic(balance: u64) -> Account {
    Account::Basic(BasicAccount {
        balance: coin(balance),
    })
}

fn blake2b(data: &[u8]) -> AnyHash {
    AnyHash::Blake2b(AnyHash32::from(
        Blake2bHasher::default().digest(data).as_bytes(),
    ))
}

/// Verifies `tx` and commits it on its own in a block, checking that the commit succeeds and
/// reverts cleanly.
fn commit(test: &TestCommitRevert, tx: Transaction) {
    tx.verify(NETWORK_ID, PROTOCOL_VERSION)
        .expect("the built transaction must verify");

    let block_state = BlockState::new(1, 1, PROTOCOL_VERSION);
    let receipts = test
        .commit_and_test(&[tx], &[], &block_state, &mut BlockLogger::empty())
        .expect("the block must commit");
    assert!(
        matches!(receipts.transactions[0], OperationReceipt::Ok(_)),
        "the transaction must succeed, got {:?}",
        receipts.transactions[0]
    );
}

fn oracle(test: &TestCommitRevert, address: &Address) -> OracleContract {
    match test.get_complete(address, None) {
        Account::Oracle(oracle) => oracle,
        account => panic!("expected an oracle contract, got {account:?}"),
    }
}

#[test]
fn built_oracle_transactions_are_accepted() {
    let payer = KeyPair::generate_default_csprng();
    let owner = KeyPair::generate_default_csprng();
    let new_owner = KeyPair::generate_default_csprng();
    let recipient = Address::from([0x42u8; 20]);
    let deposit = coin(1_000);
    let fee = coin(10);

    let test = TestCommitRevert::with_initial_state(&[(Address::from(&payer), basic(10_000))]);

    let create = TransactionBuilder::new_create_oracle(
        &payer,
        Address::from(&owner),
        4,
        deposit,
        fee,
        1,
        NETWORK_ID,
    )
    .unwrap();
    let oracle_address = create.contract_creation_address();
    commit(&test, create);
    assert_eq!(oracle(&test, &oracle_address).owner, Address::from(&owner));

    commit(
        &test,
        TransactionBuilder::new_update_oracle(
            &payer,
            &owner,
            oracle_address.clone(),
            vec![blake2b(b"state 0"), blake2b(b"state 1")],
            fee,
            1,
            NETWORK_ID,
        ),
    );
    assert_eq!(oracle(&test, &oracle_address).latest_index, Some(1));

    commit(
        &test,
        TransactionBuilder::new_change_oracle_owner(
            &payer,
            &owner,
            oracle_address.clone(),
            Address::from(&new_owner),
            fee,
            1,
            NETWORK_ID,
        ),
    );
    assert_eq!(
        oracle(&test, &oracle_address).owner,
        Address::from(&new_owner)
    );

    // Only the new owner can update the oracle now.
    commit(
        &test,
        TransactionBuilder::new_update_oracle(
            &payer,
            &new_owner,
            oracle_address.clone(),
            vec![blake2b(b"state 2")],
            fee,
            1,
            NETWORK_ID,
        ),
    );
    assert_eq!(oracle(&test, &oracle_address).latest_index, Some(2));

    commit(
        &test,
        TransactionBuilder::new_delete_oracle(
            &new_owner,
            oracle_address.clone(),
            recipient.clone(),
            deposit,
            Coin::ZERO,
            1,
            NETWORK_ID,
        )
        .unwrap(),
    );
    assert_eq!(test.get_complete(&recipient, None).balance(), deposit);
    assert!(matches!(
        test.get_complete(&oracle_address, None),
        Account::Basic(_)
    ));

    // The payer paid the deposit and four fees.
    assert_eq!(
        test.get_complete(&Address::from(&payer), None).balance(),
        coin(10_000 - 1_000 - 4 * 10)
    );
}

/// Reads all fields from the burn data:
/// `[0..20]` target address, `[20..28]` amount, `[28..36]` nonce,
/// `[36..40]` burn block height, `[40..44]` target chain ID.
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

#[test]
fn built_bridge_transactions_are_accepted() {
    let depositor = KeyPair::generate_default_csprng();
    let relayer = KeyPair::generate_default_csprng();
    let oracle_owner = KeyPair::generate_default_csprng();
    let oracle_address = Address::from([0x0Eu8; 20]);
    let bridge_address = Address::from([0x0Bu8; 20]);
    let target = Address::from([0xAAu8; 20]);
    let bridge_balance = 10_000;
    let released = 500;
    let fee = coin(10);

    let oracle = OracleContract {
        owner: Address::from(&oracle_owner),
        balance: coin(1_000),
        hash_count: 4,
        hashes: Vec::new(),
        latest_index: None,
    };
    let bridge = BridgeContract {
        owner: Address::from([0x01u8; 20]),
        oracle_address: oracle_address.clone(),
        balance: coin(bridge_balance),
        source_chain_id: SOURCE_CHAIN_ID,
        chain_config: chain_config(),
        transaction_count: 0,
    };
    let test = TestCommitRevert::with_initial_state(&[
        (oracle_address.clone(), Account::Oracle(oracle)),
        (bridge_address.clone(), Account::Bridge(bridge)),
        (Address::from(&depositor), basic(5_000)),
        (Address::from(&relayer), basic(100)),
    ]);

    commit(
        &test,
        TransactionBuilder::new_bridge_deposit(
            &depositor,
            bridge_address.clone(),
            vec![0x5Au8; 20],
            coin(2_000),
            fee,
            1,
            NETWORK_ID,
        )
        .unwrap(),
    );
    assert_eq!(
        test.get_complete(&Address::from(&depositor), None)
            .balance(),
        coin(5_000 - 2_000 - 10)
    );

    // Each oracle state commits to a single-leaf tree holding one burn transaction. The oracle
    // chains the second state onto the first.
    let burns = [
        burn_data(&target, released, 1),
        burn_data(&target, released, 2),
    ];
    commit(
        &test,
        TransactionBuilder::new_update_oracle(
            &depositor,
            &oracle_owner,
            oracle_address,
            burns.iter().map(|burn| blake2b(burn)).collect(),
            Coin::ZERO,
            1,
            NETWORK_ID,
        ),
    );

    for (index, burn) in burns.into_iter().enumerate() {
        commit(
            &test,
            TransactionBuilder::new_bridge_release(
                &relayer,
                bridge_address.clone(),
                target.clone(),
                OutgoingTransaction::new(
                    burn,
                    AnyMerkleProof::Blake2bPath(MerklePath::empty()),
                    index as u64,
                )
                .unwrap(),
                coin(released),
                fee,
                1,
                NETWORK_ID,
            )
            .unwrap(),
        );
    }

    // Releases pay the target from the bridge, and the fees from the relayer.
    assert_eq!(
        test.get_complete(&target, None).balance(),
        coin(2 * released)
    );
    assert_eq!(
        test.get_complete(&bridge_address, None).balance(),
        coin(bridge_balance + 2_000 - 2 * released)
    );
    assert_eq!(
        test.get_complete(&Address::from(&relayer), None).balance(),
        coin(100 - 2 * 10)
    );
}
