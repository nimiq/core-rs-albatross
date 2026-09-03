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

/// Divides a 32-byte big-endian EVM word by `divisor`, keeping the full 256-bit width, and
/// returns the quotient if it fits in a u64.
///
/// This exists because narrowing first and dividing afterwards throws away the whole point of the
/// division. An EVM token amount is a 256-bit word in the token's own base units; converting it to
/// luna means dividing by the decimal factor. If the word has to fit a u64 *before* that division,
/// the reachable range is capped at `u64::MAX` base units rather than `u64::MAX` luna — for wNIM's
/// 10^13 wei per luna that is a ceiling of about 18.44 NIM per burn. Dividing first lifts the
/// ceiling to `u64::MAX` luna, far above the total supply.
///
/// The division is exact byte-wise long division, so no precision is lost and no wide-integer
/// dependency is needed. Truncation of any sub-unit remainder is intentional and matches `Div`.
pub fn divide_bytes32_by_u64(bytes: &[u8; 32], divisor: u64) -> Result<u64, BridgeError> {
    if divisor == 0 {
        return Err(BridgeError::InvalidAmount);
    }
    let divisor = divisor as u128;

    let mut quotient = [0u8; 32];
    let mut remainder: u128 = 0;
    for (index, &byte) in bytes.iter().enumerate() {
        // `remainder` is always below `divisor <= u64::MAX`, so shifting it up by a byte and
        // adding one more cannot leave the u128 range.
        remainder = (remainder << 8) | byte as u128;
        // And it is below `divisor * 256`, so the quotient digit is a single byte.
        quotient[index] = (remainder / divisor) as u8;
        remainder %= divisor;
    }

    // The *quotient* is what has to fit a u64 now, not the raw word.
    bytes32_to_u64(&quotient)
}

/// Performs checked power operation with overflow detection.
///
/// Computes a^b with overflow checking, inspired by EVM behavior.
pub fn checked_pow_u256(base: u64, exponent: u32) -> Result<u64, BridgeError> {
    base.checked_pow(exponent).ok_or(BridgeError::InvalidAmount)
}
