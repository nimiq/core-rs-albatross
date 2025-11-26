use nimiq_hash::{Keccak256Hash, Keccak256Hasher};
use nimiq_keys::Address;
use nimiq_primitives::{coin::Coin, networks::NetworkId};
use nimiq_rpc_interface::types::MerklePathData;
use nimiq_transaction::historic_transaction::{HistoricTransaction, HistoricTransactionData};
use nimiq_utils::merkle::MerklePath;

#[test]
fn test_merkle_path_data_creation() {
    // Create some test data
    let values = vec!["tx1", "tx2", "tx3", "tx4"];

    // Create a Merkle path for the second transaction
    let path = MerklePath::<Keccak256Hash>::new::<Keccak256Hasher, &str>(&values, &values[1]);

    // Create a test historic transaction
    let historic_tx = HistoricTransaction {
        network_id: NetworkId::UnitAlbatross,
        block_number: 100,
        block_time: 1234567890,
        data: HistoricTransactionData::Basic(nimiq_transaction::ExecutedTransaction::Ok(
            nimiq_transaction::Transaction::new_basic(
                Address::from([0u8; 20]),
                Address::from([1u8; 20]),
                Coin::from_u64_unchecked(1000),
                Coin::from_u64_unchecked(10),
                1,
                NetworkId::UnitAlbatross,
            ),
        )),
    };

    // Convert to MerklePathData
    let merkle_path_data = MerklePathData::from_path(path.clone(), historic_tx, Some(200));

    assert!(merkle_path_data.is_some());
    let data = merkle_path_data.unwrap();

    // Verify the structure
    assert_eq!(data.hashes.len(), path.len());

    // Verify all hashes start with "0x"
    for hash in &data.hashes {
        assert!(hash.starts_with("0x"));
        assert_eq!(hash.len(), 66); // "0x" + 64 hex chars for 32 bytes
    }
}

#[test]
fn test_merkle_path_data_empty_path() {
    // Create an empty path
    let path = MerklePath::<Keccak256Hash>::empty();

    // Create a test historic transaction
    let historic_tx = HistoricTransaction {
        network_id: NetworkId::UnitAlbatross,
        block_number: 100,
        block_time: 1234567890,
        data: HistoricTransactionData::Basic(nimiq_transaction::ExecutedTransaction::Ok(
            nimiq_transaction::Transaction::new_basic(
                Address::from([0u8; 20]),
                Address::from([1u8; 20]),
                Coin::from_u64_unchecked(1000),
                Coin::from_u64_unchecked(10),
                1,
                NetworkId::UnitAlbatross,
            ),
        )),
    };

    // Convert to MerklePathData
    let merkle_path_data = MerklePathData::from_path(path, historic_tx, Some(200));

    assert!(merkle_path_data.is_some());
    let data = merkle_path_data.unwrap();

    // Verify empty path
    assert_eq!(data.hashes.len(), 0);
}

#[test]
fn test_merkle_path_data_serialization() {
    // Create some test data
    let values = vec!["tx1", "tx2", "tx3"];

    // Create a Merkle path
    let path = MerklePath::<Keccak256Hash>::new::<Keccak256Hasher, &str>(&values, &values[0]);

    // Create a test historic transaction
    let historic_tx = HistoricTransaction {
        network_id: NetworkId::UnitAlbatross,
        block_number: 100,
        block_time: 1234567890,
        data: HistoricTransactionData::Basic(nimiq_transaction::ExecutedTransaction::Ok(
            nimiq_transaction::Transaction::new_basic(
                Address::from([0u8; 20]),
                Address::from([1u8; 20]),
                Coin::from_u64_unchecked(1000),
                Coin::from_u64_unchecked(10),
                1,
                NetworkId::UnitAlbatross,
            ),
        )),
    };

    // Convert to MerklePathData
    let merkle_path_data = MerklePathData::from_path(path, historic_tx, Some(200)).unwrap();

    // Test serialization
    let json = serde_json::to_string(&merkle_path_data).unwrap();
    assert!(json.contains("hashes"));
    assert!(json.contains("transaction"));

    // Test deserialization
    let deserialized: MerklePathData = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized.hashes, merkle_path_data.hashes);
}
