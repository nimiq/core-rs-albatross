use nimiq_account::{Account, BridgeContract};
use nimiq_keys::Address;
use nimiq_primitives::{account::AccountType, coin::Coin};
use nimiq_transaction::{
    account::htlc_contract::{AnyHash, AnyHash32},
    bridge_contract::{AddressFormat, ChainConfig, Endianness},
};

#[test]
fn test_bridge_account_type() {
    let chain_config = ChainConfig {
        chain_id: 1,
        hash_function: AnyHash::Blake2b(AnyHash32::default()),
        address_format: AddressFormat::Nimiq,
        endianness: Endianness::LittleEndian,
        block_time: std::time::Duration::from_secs(60),
        validation_program: nimiq_transaction::bridge_contract::ValidationProgram::empty(),
        max_proof_depth: 64,
    };

    let bridge_contract = BridgeContract {
        owner: Address::from([1u8; 20]),
        oracle_address: Address::from([2u8; 20]),
        balance: Coin::from_u64_unchecked(1000),
        source_chain_id: 1,
        chain_config: chain_config.clone(),
        transaction_count: 0,
    };

    let account = Account::Bridge(bridge_contract.clone());

    assert_eq!(account.account_type(), AccountType::Bridge);
    assert_eq!(account.balance(), Coin::from_u64_unchecked(1000));

    // Verify the bridge contract fields
    if let Account::Bridge(bridge) = account {
        assert_eq!(bridge.owner, Address::from([1u8; 20]));
        assert_eq!(bridge.oracle_address, Address::from([2u8; 20]));
        assert_eq!(bridge.source_chain_id, 1);
        assert_eq!(bridge.chain_config.chain_id, 1);
        assert_eq!(bridge.transaction_count, 0);
    } else {
        panic!("Expected Bridge account");
    }
}

#[test]
fn test_bridge_account_serialization() {
    let chain_config = ChainConfig {
        chain_id: 1,
        hash_function: AnyHash::Blake2b(AnyHash32::default()),
        address_format: AddressFormat::Nimiq,
        endianness: Endianness::LittleEndian,
        block_time: std::time::Duration::from_secs(60),
        validation_program: nimiq_transaction::bridge_contract::ValidationProgram::empty(),
        max_proof_depth: 64,
    };

    let bridge_contract = BridgeContract {
        owner: Address::from([1u8; 20]),
        oracle_address: Address::from([2u8; 20]),
        balance: Coin::from_u64_unchecked(1000),
        source_chain_id: 1,
        chain_config,
        transaction_count: 5,
    };

    let account = Account::Bridge(bridge_contract);

    // Test serialization and deserialization
    let serialized = nimiq_serde::Serialize::serialize_to_vec(&account);
    let deserialized: Account =
        nimiq_serde::Deserialize::deserialize_from_vec(&serialized).unwrap();

    assert_eq!(account, deserialized);
}
