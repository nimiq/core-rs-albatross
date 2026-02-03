// Unit tests for decode arithmetic operations
//
// Tests for checked arithmetic operations inspired by EVM behavior,
// including addition, subtraction, multiplication, division, modulo,
// and power operations with overflow/underflow detection.

use nimiq_transaction::bridge_contract::{
    decode_arithmetic::{
        bytes32_to_u64, checked_add_u256, checked_div_u256, checked_mod_u256, checked_mul_u256,
        checked_pow_u256, checked_sub_u256, u64_to_bytes32,
    },
    BridgeError,
};

// ============================================================================
// Addition Tests
// ============================================================================

/// Test checked addition with valid operands (inspired by EVM arithmetic)
#[test]
fn test_decode_arithmetic_checked_add_valid() {
    let result = checked_add_u256(100, 200);
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), 300);
}

/// Test checked addition with zero
#[test]
fn test_decode_arithmetic_checked_add_with_zero() {
    assert_eq!(checked_add_u256(0, 0).unwrap(), 0);
    assert_eq!(checked_add_u256(100, 0).unwrap(), 100);
    assert_eq!(checked_add_u256(0, 100).unwrap(), 100);
}

/// Test checked addition overflow handling
#[test]
fn test_decode_arithmetic_checked_add_overflow() {
    let result = checked_add_u256(u64::MAX, 1);
    assert!(result.is_err());
    assert!(matches!(result.unwrap_err(), BridgeError::InvalidAmount));
}

/// Test checked addition near max value
#[test]
fn test_decode_arithmetic_checked_add_near_max() {
    let result = checked_add_u256(u64::MAX - 1, 1);
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), u64::MAX);
}

// ============================================================================
// Subtraction Tests
// ============================================================================

/// Test checked subtraction with valid operands
#[test]
fn test_decode_arithmetic_checked_sub_valid() {
    let result = checked_sub_u256(300, 100);
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), 200);
}

/// Test checked subtraction with zero
#[test]
fn test_decode_arithmetic_checked_sub_with_zero() {
    assert_eq!(checked_sub_u256(100, 0).unwrap(), 100);
    assert_eq!(checked_sub_u256(0, 0).unwrap(), 0);
}

/// Test checked subtraction underflow handling
#[test]
fn test_decode_arithmetic_checked_sub_underflow() {
    let result = checked_sub_u256(100, 200);
    assert!(result.is_err());
    assert!(matches!(result.unwrap_err(), BridgeError::InvalidAmount));
}

/// Test checked subtraction at boundary
#[test]
fn test_decode_arithmetic_checked_sub_boundary() {
    let result = checked_sub_u256(u64::MAX, u64::MAX);
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), 0);
}

// ============================================================================
// Multiplication Tests
// ============================================================================

/// Test checked multiplication with valid operands
#[test]
fn test_decode_arithmetic_checked_mul_valid() {
    let result = checked_mul_u256(10, 20);
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), 200);
}

/// Test checked multiplication with zero
#[test]
fn test_decode_arithmetic_checked_mul_with_zero() {
    assert_eq!(checked_mul_u256(0, 0).unwrap(), 0);
    assert_eq!(checked_mul_u256(100, 0).unwrap(), 0);
    assert_eq!(checked_mul_u256(0, 100).unwrap(), 0);
}

/// Test checked multiplication with one
#[test]
fn test_decode_arithmetic_checked_mul_with_one() {
    assert_eq!(checked_mul_u256(1, 1).unwrap(), 1);
    assert_eq!(checked_mul_u256(100, 1).unwrap(), 100);
    assert_eq!(checked_mul_u256(1, 100).unwrap(), 100);
}

/// Test checked multiplication overflow handling
#[test]
fn test_decode_arithmetic_checked_mul_overflow() {
    let result = checked_mul_u256(u64::MAX, 2);
    assert!(result.is_err());
    assert!(matches!(result.unwrap_err(), BridgeError::InvalidAmount));
}

/// Test checked multiplication near max value
#[test]
fn test_decode_arithmetic_checked_mul_near_max() {
    let result = checked_mul_u256(u64::MAX / 2, 2);
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), u64::MAX - 1);
}

// ============================================================================
// Division Tests
// ============================================================================

/// Test checked division with valid operands
#[test]
fn test_decode_arithmetic_checked_div_valid() {
    let result = checked_div_u256(200, 10);
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), 20);
}

/// Test checked division by one
#[test]
fn test_decode_arithmetic_checked_div_by_one() {
    assert_eq!(checked_div_u256(100, 1).unwrap(), 100);
    assert_eq!(checked_div_u256(u64::MAX, 1).unwrap(), u64::MAX);
}

/// Test checked division by zero handling
#[test]
fn test_decode_arithmetic_checked_div_by_zero() {
    let result = checked_div_u256(100, 0);
    assert!(result.is_err());
    assert!(matches!(result.unwrap_err(), BridgeError::InvalidAmount));
}

/// Test checked division with truncation
#[test]
fn test_decode_arithmetic_checked_div_truncation() {
    let result = checked_div_u256(100, 3);
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), 33); // Integer division truncates
}

// ============================================================================
// Modulo Tests
// ============================================================================

/// Test checked modulo with valid operands
#[test]
fn test_decode_arithmetic_checked_mod_valid() {
    let result = checked_mod_u256(100, 30);
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), 10);
}

/// Test checked modulo by one
#[test]
fn test_decode_arithmetic_checked_mod_by_one() {
    assert_eq!(checked_mod_u256(100, 1).unwrap(), 0);
    assert_eq!(checked_mod_u256(u64::MAX, 1).unwrap(), 0);
}

/// Test checked modulo by zero handling
#[test]
fn test_decode_arithmetic_checked_mod_by_zero() {
    let result = checked_mod_u256(100, 0);
    assert!(result.is_err());
    assert!(matches!(result.unwrap_err(), BridgeError::InvalidAmount));
}

/// Test checked modulo with equal operands
#[test]
fn test_decode_arithmetic_checked_mod_equal_operands() {
    assert_eq!(checked_mod_u256(100, 100).unwrap(), 0);
    assert_eq!(checked_mod_u256(u64::MAX, u64::MAX).unwrap(), 0);
}

// ============================================================================
// Bytes Conversion Tests (32-byte format)
// ============================================================================

/// Test u64 to 32-byte conversion
#[test]
fn test_decode_arithmetic_u64_to_bytes32() {
    let value = 12345u64;
    let bytes = u64_to_bytes32(value);

    // Should be 32 bytes
    assert_eq!(bytes.len(), 32);

    // First 24 bytes should be zero
    for i in 0..24 {
        assert_eq!(bytes[i], 0);
    }

    // Last 8 bytes should contain the value in big-endian
    let expected_bytes = value.to_be_bytes();
    assert_eq!(&bytes[24..32], &expected_bytes);
}

/// Test u64 to 32-byte conversion with zero
#[test]
fn test_decode_arithmetic_u64_to_bytes32_zero() {
    let bytes = u64_to_bytes32(0);
    assert_eq!(bytes.len(), 32);
    for byte in bytes.iter() {
        assert_eq!(*byte, 0);
    }
}

/// Test u64 to 32-byte conversion with max value
#[test]
fn test_decode_arithmetic_u64_to_bytes32_max() {
    let bytes = u64_to_bytes32(u64::MAX);
    assert_eq!(bytes.len(), 32);

    // First 24 bytes should be zero
    for i in 0..24 {
        assert_eq!(bytes[i], 0);
    }

    // Last 8 bytes should be all 0xFF
    for i in 24..32 {
        assert_eq!(bytes[i], 0xFF);
    }
}

/// Test 32-byte to u64 conversion
#[test]
fn test_decode_arithmetic_bytes32_to_u64() {
    let value = 12345u64;
    let bytes = u64_to_bytes32(value);
    let result = bytes32_to_u64(&bytes);

    assert!(result.is_ok());
    assert_eq!(result.unwrap(), value);
}

/// Test 32-byte to u64 conversion with zero
#[test]
fn test_decode_arithmetic_bytes32_to_u64_zero() {
    let bytes = [0u8; 32];
    let result = bytes32_to_u64(&bytes);

    assert!(result.is_ok());
    assert_eq!(result.unwrap(), 0);
}

/// Test 32-byte to u64 conversion with max value
#[test]
fn test_decode_arithmetic_bytes32_to_u64_max() {
    let mut bytes = [0u8; 32];
    bytes[24..32].copy_from_slice(&u64::MAX.to_be_bytes());
    let result = bytes32_to_u64(&bytes);

    assert!(result.is_ok());
    assert_eq!(result.unwrap(), u64::MAX);
}

/// Test 32-byte to u64 conversion with overflow (value too large)
#[test]
fn test_decode_arithmetic_bytes32_to_u64_overflow() {
    let mut bytes = [0u8; 32];
    bytes[23] = 1; // Set a byte in the upper 24 bytes
    let result = bytes32_to_u64(&bytes);

    assert!(result.is_err());
    assert!(matches!(result.unwrap_err(), BridgeError::InvalidAmount));
}

/// Test bytes round-trip conversion
#[test]
fn test_decode_arithmetic_bytes_round_trip() {
    let values = [0u64, 1, 100, 12345, u64::MAX / 2, u64::MAX];

    for value in values.iter() {
        let bytes = u64_to_bytes32(*value);
        let result = bytes32_to_u64(&bytes);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), *value);
    }
}

// ============================================================================
// Power Operation Tests
// ============================================================================

/// Test checked power operation with valid operands
#[test]
fn test_decode_arithmetic_checked_pow_valid() {
    let result = checked_pow_u256(2, 10);
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), 1024);
}

/// Test checked power operation with zero exponent
#[test]
fn test_decode_arithmetic_checked_pow_zero_exponent() {
    assert_eq!(checked_pow_u256(100, 0).unwrap(), 1);
    assert_eq!(checked_pow_u256(0, 0).unwrap(), 1);
}

/// Test checked power operation with one exponent
#[test]
fn test_decode_arithmetic_checked_pow_one_exponent() {
    assert_eq!(checked_pow_u256(100, 1).unwrap(), 100);
    assert_eq!(checked_pow_u256(u64::MAX, 1).unwrap(), u64::MAX);
}

/// Test checked power operation overflow handling
#[test]
fn test_decode_arithmetic_checked_pow_overflow() {
    let result = checked_pow_u256(2, 64);
    assert!(result.is_err());
    assert!(matches!(result.unwrap_err(), BridgeError::InvalidAmount));
}

/// Test checked power operation with large base
#[test]
fn test_decode_arithmetic_checked_pow_large_base() {
    let result = checked_pow_u256(u64::MAX, 2);
    assert!(result.is_err());
    assert!(matches!(result.unwrap_err(), BridgeError::InvalidAmount));
}

// ============================================================================
// Integration Tests
// ============================================================================

/// Test compatibility with Ethereum transaction formats - basic structure
#[test]
fn test_decode_arithmetic_ethereum_transaction_compatibility() {
    // Simulate Ethereum transaction value encoding
    let eth_value = 1_000_000_000_000_000_000u64; // 1 ETH in wei
    let evm_bytes = u64_to_bytes32(eth_value);

    // Verify it can be decoded back
    let decoded = bytes32_to_u64(&evm_bytes);
    assert!(decoded.is_ok());
    assert_eq!(decoded.unwrap(), eth_value);
}

/// Test arithmetic operations sequence (add, mul, div)
#[test]
fn test_decode_arithmetic_operations_sequence() {
    // Simulate a complex calculation: (100 + 50) * 2 / 3
    let step1 = checked_add_u256(100, 50).unwrap();
    assert_eq!(step1, 150);

    let step2 = checked_mul_u256(step1, 2).unwrap();
    assert_eq!(step2, 300);

    let step3 = checked_div_u256(step2, 3).unwrap();
    assert_eq!(step3, 100);
}

/// Test arithmetic with typical blockchain values
#[test]
fn test_decode_arithmetic_blockchain_values() {
    // Test with typical blockchain amounts (in smallest units)
    let amount1 = 1_000_000_000u64; // 1 billion units
    let amount2 = 500_000_000u64; // 500 million units

    let sum = checked_add_u256(amount1, amount2).unwrap();
    assert_eq!(sum, 1_500_000_000);

    let diff = checked_sub_u256(amount1, amount2).unwrap();
    assert_eq!(diff, 500_000_000);
}
