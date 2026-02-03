// Decode arithmetic operations module
//
// This module provides arithmetic operations with overflow/underflow checking,
// inspired by EVM (Ethereum Virtual Machine) arithmetic behavior where operations
// revert on overflow/underflow rather than wrapping.
//
// These operations are used in the validation program execution to safely perform
// calculations on decoded transaction data from various blockchain sources.

use super::BridgeError;

/// Performs checked addition with overflow detection.
///
/// Returns an error if the addition would overflow, inspired by EVM behavior
/// where arithmetic operations revert on overflow.
pub fn checked_add_u256(a: u64, b: u64) -> Result<u64, BridgeError> {
    a.checked_add(b).ok_or(BridgeError::InvalidAmount)
}

/// Performs checked subtraction with underflow detection.
///
/// Returns an error if the subtraction would underflow, inspired by EVM behavior
/// where arithmetic operations revert on underflow.
pub fn checked_sub_u256(a: u64, b: u64) -> Result<u64, BridgeError> {
    a.checked_sub(b).ok_or(BridgeError::InvalidAmount)
}

/// Performs checked multiplication with overflow detection.
///
/// Returns an error if the multiplication would overflow, inspired by EVM behavior.
pub fn checked_mul_u256(a: u64, b: u64) -> Result<u64, BridgeError> {
    a.checked_mul(b).ok_or(BridgeError::InvalidAmount)
}

/// Performs checked division with zero-check.
///
/// Returns an error if dividing by zero, inspired by EVM behavior where
/// division by zero causes a revert.
pub fn checked_div_u256(a: u64, b: u64) -> Result<u64, BridgeError> {
    if b == 0 {
        return Err(BridgeError::InvalidAmount);
    }
    Ok(a / b)
}

/// Performs checked modulo operation with zero-check.
///
/// Returns an error if the modulus is zero, inspired by EVM behavior.
pub fn checked_mod_u256(a: u64, b: u64) -> Result<u64, BridgeError> {
    if b == 0 {
        return Err(BridgeError::InvalidAmount);
    }
    Ok(a % b)
}

/// Converts a u64 value to 32-byte big-endian format.
///
/// This format is compatible with EVM 256-bit words. The function converts
/// a u64 value to a 32-byte array with proper zero-padding.
pub fn u64_to_bytes32(value: u64) -> [u8; 32] {
    let mut bytes = [0u8; 32];
    // Place the u64 in the last 8 bytes (big-endian)
    bytes[24..32].copy_from_slice(&value.to_be_bytes());
    bytes
}

/// Converts 32-byte big-endian format to u64.
///
/// Extracts a u64 value from a 32-byte word (EVM-compatible format).
/// Returns an error if the value is too large to fit in a u64.
pub fn bytes32_to_u64(bytes: &[u8; 32]) -> Result<u64, BridgeError> {
    // Check that the first 24 bytes are zero (value fits in u64)
    for &byte in &bytes[0..24] {
        if byte != 0 {
            return Err(BridgeError::InvalidAmount);
        }
    }
    // Extract the last 8 bytes as u64
    let mut value_bytes = [0u8; 8];
    value_bytes.copy_from_slice(&bytes[24..32]);
    Ok(u64::from_be_bytes(value_bytes))
}

/// Performs checked power operation with overflow detection.
///
/// Computes a^b with overflow checking, inspired by EVM behavior.
pub fn checked_pow_u256(base: u64, exponent: u32) -> Result<u64, BridgeError> {
    base.checked_pow(exponent).ok_or(BridgeError::InvalidAmount)
}
