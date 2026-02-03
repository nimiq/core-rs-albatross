// Tests for HashType functionality used in bridge-oracle hash validation
use std::collections::HashMap;

use nimiq_transaction::{account::htlc_contract::AnyHash32, AnyHash, HashType};

/// Test HashType extraction from AnyHash variants
#[test]
fn test_hash_type_from_any_hash() {
    // Test Blake2b
    let blake2b_hash = AnyHash::Blake2b(AnyHash32::default());
    assert_eq!(HashType::from_any_hash(&blake2b_hash), HashType::Blake2b);
    assert_eq!(HashType::from_any_hash(&blake2b_hash).name(), "Blake2b");

    // Test SHA256
    let sha256_hash = AnyHash::Sha256(AnyHash32::default());
    assert_eq!(HashType::from_any_hash(&sha256_hash), HashType::Sha256);
    assert_eq!(HashType::from_any_hash(&sha256_hash).name(), "SHA256");

    // Test Keccak256
    let keccak256_hash = AnyHash::Keccak256(AnyHash32::default());
    assert_eq!(
        HashType::from_any_hash(&keccak256_hash),
        HashType::Keccak256
    );
    assert_eq!(HashType::from_any_hash(&keccak256_hash).name(), "Keccak256");
}

/// Test HashType equality comparison
#[test]
fn test_hash_type_equality() {
    let blake2b = HashType::Blake2b;
    let sha256 = HashType::Sha256;
    let keccak256 = HashType::Keccak256;

    // Same types should be equal
    assert_eq!(blake2b, HashType::Blake2b);
    assert_eq!(sha256, HashType::Sha256);
    assert_eq!(keccak256, HashType::Keccak256);

    // Different types should not be equal
    assert_ne!(blake2b, sha256);
    assert_ne!(blake2b, keccak256);
    assert_ne!(sha256, keccak256);
}

/// Test HashType comparison from different AnyHash instances
#[test]
fn test_hash_type_comparison_from_any_hash() {
    let blake2b_hash1 = AnyHash::Blake2b(AnyHash32::default());
    let blake2b_hash2 = AnyHash::Blake2b(AnyHash32::from([1u8; 32]));
    let sha256_hash = AnyHash::Sha256(AnyHash32::default());

    // Same hash types should match even with different values
    assert_eq!(
        HashType::from_any_hash(&blake2b_hash1),
        HashType::from_any_hash(&blake2b_hash2)
    );

    // Different hash types should not match
    assert_ne!(
        HashType::from_any_hash(&blake2b_hash1),
        HashType::from_any_hash(&sha256_hash)
    );
}

/// Test HashType names
#[test]
fn test_hash_type_names() {
    assert_eq!(HashType::Blake2b.name(), "Blake2b");
    assert_eq!(HashType::Sha256.name(), "SHA256");
    assert_eq!(HashType::Sha512.name(), "SHA512");
    assert_eq!(HashType::Keccak256.name(), "Keccak256");
}

/// Test that HashType can be used for validation logic
#[test]
fn test_hash_type_validation_logic() {
    // Simulate oracle with Keccak256
    let oracle_hash = AnyHash::Keccak256(AnyHash32::default());
    let oracle_type = HashType::from_any_hash(&oracle_hash);

    // Bridge with matching Keccak256 should pass
    let bridge_hash_matching = AnyHash::Keccak256(AnyHash32::from([1u8; 32]));
    let bridge_type_matching = HashType::from_any_hash(&bridge_hash_matching);
    assert_eq!(oracle_type, bridge_type_matching);

    // Bridge with mismatched Blake2b should fail
    let bridge_hash_mismatched = AnyHash::Blake2b(AnyHash32::default());
    let bridge_type_mismatched = HashType::from_any_hash(&bridge_hash_mismatched);
    assert_ne!(oracle_type, bridge_type_mismatched);
}

/// Test all hash type combinations for mismatch detection
#[test]
fn test_all_hash_type_mismatches() {
    let hash_types = vec![
        (AnyHash::Blake2b(AnyHash32::default()), "Blake2b"),
        (AnyHash::Sha256(AnyHash32::default()), "SHA256"),
        (AnyHash::Keccak256(AnyHash32::default()), "Keccak256"),
    ];

    for (i, (hash1, name1)) in hash_types.iter().enumerate() {
        for (j, (hash2, name2)) in hash_types.iter().enumerate() {
            let type1 = HashType::from_any_hash(hash1);
            let type2 = HashType::from_any_hash(hash2);

            if i == j {
                // Same type should match
                assert_eq!(type1, type2, "{} should match {}", name1, name2);
            } else {
                // Different types should not match
                assert_ne!(type1, type2, "{} should not match {}", name1, name2);
            }
        }
    }
}

/// Test HashType ordering (for use in sorted collections)
#[test]
fn test_hash_type_ordering() {
    let mut types = vec![
        HashType::Keccak256,
        HashType::Blake2b,
        HashType::Sha512,
        HashType::Sha256,
    ];

    types.sort();

    // Verify types can be sorted (exact order doesn't matter, just that it's consistent)
    assert_eq!(types.len(), 4);
    assert!(types.contains(&HashType::Blake2b));
    assert!(types.contains(&HashType::Sha256));
    assert!(types.contains(&HashType::Sha512));
    assert!(types.contains(&HashType::Keccak256));
}

/// Test HashType can be used as a key in hash maps
#[test]
fn test_hash_type_as_map_key() {
    let mut map = HashMap::new();
    map.insert(HashType::Blake2b, "Blake2b hash");
    map.insert(HashType::Sha256, "SHA256 hash");
    map.insert(HashType::Keccak256, "Keccak256 hash");

    assert_eq!(map.get(&HashType::Blake2b), Some(&"Blake2b hash"));
    assert_eq!(map.get(&HashType::Sha256), Some(&"SHA256 hash"));
    assert_eq!(map.get(&HashType::Keccak256), Some(&"Keccak256 hash"));
    assert_eq!(map.get(&HashType::Sha512), None);
}

/// Test that HashType correctly identifies chain-specific hash algorithms
#[test]
fn test_chain_specific_hash_identification() {
    // Ethereum uses Keccak256
    let ethereum_hash = AnyHash::Keccak256(AnyHash32::default());
    assert_eq!(HashType::from_any_hash(&ethereum_hash), HashType::Keccak256);
    assert_eq!(HashType::from_any_hash(&ethereum_hash).name(), "Keccak256");

    // Bitcoin uses SHA256
    let bitcoin_hash = AnyHash::Sha256(AnyHash32::default());
    assert_eq!(HashType::from_any_hash(&bitcoin_hash), HashType::Sha256);
    assert_eq!(HashType::from_any_hash(&bitcoin_hash).name(), "SHA256");

    // Nimiq uses Blake2b
    let nimiq_hash = AnyHash::Blake2b(AnyHash32::default());
    assert_eq!(HashType::from_any_hash(&nimiq_hash), HashType::Blake2b);
    assert_eq!(HashType::from_any_hash(&nimiq_hash).name(), "Blake2b");
}
