use nimiq_transaction::bridge_contract::{
    endianness_conversion::{
        convert_address, convert_bytes, convert_u32, convert_u64, encode_u32, encode_u64,
        parse_u32, parse_u64,
    },
    BridgeError, Endianness,
};

// ============================================================================
// u32 Conversion Tests
// ============================================================================

/// Test little-endian to big-endian conversion for u32
#[test]
fn test_convert_u32_little_to_big() {
    let value = 0x12345678u32;
    let result = convert_u32(value, Endianness::LittleEndian, Endianness::BigEndian);
    assert_eq!(result, 0x78563412u32);
}

/// Test big-endian to little-endian conversion for u32
#[test]
fn test_convert_u32_big_to_little() {
    let value = 0x12345678u32;
    let result = convert_u32(value, Endianness::BigEndian, Endianness::LittleEndian);
    assert_eq!(result, 0x78563412u32);
}

/// Test same endianness conversion for u32 (no change)
#[test]
fn test_convert_u32_same_endianness() {
    let value = 0x12345678u32;

    let result_le = convert_u32(value, Endianness::LittleEndian, Endianness::LittleEndian);
    assert_eq!(result_le, value);

    let result_be = convert_u32(value, Endianness::BigEndian, Endianness::BigEndian);
    assert_eq!(result_be, value);
}

/// Test endianness conversion for u32 with zero
#[test]
fn test_convert_u32_zero() {
    let value = 0u32;
    let result = convert_u32(value, Endianness::LittleEndian, Endianness::BigEndian);
    assert_eq!(result, 0);
}

/// Test endianness conversion for u32 with max value
#[test]
fn test_convert_u32_max() {
    let value = u32::MAX;
    let result = convert_u32(value, Endianness::LittleEndian, Endianness::BigEndian);
    assert_eq!(result, u32::MAX); // All bits set, so swap doesn't change value
}

// ============================================================================
// u64 Conversion Tests
// ============================================================================

/// Test little-endian to big-endian conversion for u64
#[test]
fn test_convert_u64_little_to_big() {
    let value = 0x123456789ABCDEF0u64;
    let result = convert_u64(value, Endianness::LittleEndian, Endianness::BigEndian);
    assert_eq!(result, 0xF0DEBC9A78563412u64);
}

/// Test big-endian to little-endian conversion for u64
#[test]
fn test_convert_u64_big_to_little() {
    let value = 0x123456789ABCDEF0u64;
    let result = convert_u64(value, Endianness::BigEndian, Endianness::LittleEndian);
    assert_eq!(result, 0xF0DEBC9A78563412u64);
}

/// Test same endianness conversion for u64 (no change)
#[test]
fn test_convert_u64_same_endianness() {
    let value = 0x123456789ABCDEF0u64;

    let result_le = convert_u64(value, Endianness::LittleEndian, Endianness::LittleEndian);
    assert_eq!(result_le, value);

    let result_be = convert_u64(value, Endianness::BigEndian, Endianness::BigEndian);
    assert_eq!(result_be, value);
}

/// Test endianness conversion for u64 with zero
#[test]
fn test_convert_u64_zero() {
    let value = 0u64;
    let result = convert_u64(value, Endianness::LittleEndian, Endianness::BigEndian);
    assert_eq!(result, 0);
}

/// Test endianness conversion for u64 with max value
#[test]
fn test_convert_u64_max() {
    let value = u64::MAX;
    let result = convert_u64(value, Endianness::LittleEndian, Endianness::BigEndian);
    assert_eq!(result, u64::MAX);
}

// ============================================================================
// Byte Slice Conversion Tests
// ============================================================================

/// Test byte slice conversion from little to big endian
#[test]
fn test_convert_bytes_little_to_big() {
    let data = vec![0x01, 0x02, 0x03, 0x04];
    let result = convert_bytes(&data, Endianness::LittleEndian, Endianness::BigEndian);
    assert_eq!(result, vec![0x04, 0x03, 0x02, 0x01]);
}

/// Test byte slice conversion from big to little endian
#[test]
fn test_convert_bytes_big_to_little() {
    let data = vec![0x01, 0x02, 0x03, 0x04];
    let result = convert_bytes(&data, Endianness::BigEndian, Endianness::LittleEndian);
    assert_eq!(result, vec![0x04, 0x03, 0x02, 0x01]);
}

/// Test byte slice conversion with same endianness
#[test]
fn test_convert_bytes_same_endianness() {
    let data = vec![0x01, 0x02, 0x03, 0x04];

    let result_le = convert_bytes(&data, Endianness::LittleEndian, Endianness::LittleEndian);
    assert_eq!(result_le, data);

    let result_be = convert_bytes(&data, Endianness::BigEndian, Endianness::BigEndian);
    assert_eq!(result_be, data);
}

/// Test byte slice conversion with empty data
#[test]
fn test_convert_bytes_empty() {
    let data: Vec<u8> = vec![];
    let result = convert_bytes(&data, Endianness::LittleEndian, Endianness::BigEndian);
    let expected: Vec<u8> = vec![];
    assert_eq!(result, expected);
}

/// Test byte slice conversion with single byte
#[test]
fn test_convert_bytes_single_byte() {
    let data = vec![0x42];
    let result = convert_bytes(&data, Endianness::LittleEndian, Endianness::BigEndian);
    assert_eq!(result, vec![0x42]);
}

// ============================================================================
// Address Conversion Tests
// ============================================================================

/// Test address conversion (20 bytes) from little to big endian
#[test]
fn test_convert_address_valid() {
    let address_bytes = [
        0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F,
        0x10, 0x11, 0x12, 0x13, 0x14,
    ];

    let result = convert_address(
        &address_bytes,
        Endianness::LittleEndian,
        Endianness::BigEndian,
    );
    assert!(result.is_ok());

    let converted = result.unwrap();
    assert_eq!(converted.len(), 20);
    assert_eq!(converted[0], 0x14);
    assert_eq!(converted[19], 0x01);
}

/// Test address conversion with invalid length
#[test]
fn test_convert_address_invalid_length() {
    let short_address = vec![0x01, 0x02, 0x03];
    let result = convert_address(
        &short_address,
        Endianness::LittleEndian,
        Endianness::BigEndian,
    );

    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        BridgeError::InvalidAddress(_)
    ));
}

/// Test address conversion with same endianness
#[test]
fn test_convert_address_same_endianness() {
    let address_bytes = [0x01; 20];
    let result = convert_address(
        &address_bytes,
        Endianness::LittleEndian,
        Endianness::LittleEndian,
    );

    assert!(result.is_ok());
    assert_eq!(result.unwrap(), address_bytes.to_vec());
}

// ============================================================================
// u32 Parsing Tests
// ============================================================================

/// Test parsing u32 from little-endian bytes
#[test]
fn test_parse_u32_little_endian() {
    let bytes = [0x78, 0x56, 0x34, 0x12]; // 0x12345678 in little-endian
    let result = parse_u32(&bytes, Endianness::LittleEndian);

    assert!(result.is_ok());
    assert_eq!(result.unwrap(), 0x12345678u32);
}

/// Test parsing u32 from big-endian bytes
#[test]
fn test_parse_u32_big_endian() {
    let bytes = [0x12, 0x34, 0x56, 0x78]; // 0x12345678 in big-endian
    let result = parse_u32(&bytes, Endianness::BigEndian);

    assert!(result.is_ok());
    assert_eq!(result.unwrap(), 0x12345678u32);
}

/// Test parsing u32 with invalid length
#[test]
fn test_parse_u32_invalid_length() {
    let bytes = [0x01, 0x02, 0x03]; // Only 3 bytes
    let result = parse_u32(&bytes, Endianness::LittleEndian);

    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        BridgeError::InvalidDataLength
    ));
}

// ============================================================================
// u64 Parsing Tests
// ============================================================================

/// Test parsing u64 from little-endian bytes
#[test]
fn test_parse_u64_little_endian() {
    let bytes = [0xF0, 0xDE, 0xBC, 0x9A, 0x78, 0x56, 0x34, 0x12]; // 0x123456789ABCDEF0 in LE
    let result = parse_u64(&bytes, Endianness::LittleEndian);

    assert!(result.is_ok());
    assert_eq!(result.unwrap(), 0x123456789ABCDEF0u64);
}

/// Test parsing u64 from big-endian bytes
#[test]
fn test_parse_u64_big_endian() {
    let bytes = [0x12, 0x34, 0x56, 0x78, 0x9A, 0xBC, 0xDE, 0xF0]; // 0x123456789ABCDEF0 in BE
    let result = parse_u64(&bytes, Endianness::BigEndian);

    assert!(result.is_ok());
    assert_eq!(result.unwrap(), 0x123456789ABCDEF0u64);
}

/// Test parsing u64 with invalid length
#[test]
fn test_parse_u64_invalid_length() {
    let bytes = [0x01, 0x02, 0x03, 0x04, 0x05]; // Only 5 bytes
    let result = parse_u64(&bytes, Endianness::LittleEndian);

    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        BridgeError::InvalidDataLength
    ));
}

// ============================================================================
// u32 Encoding Tests
// ============================================================================

/// Test encoding u32 to little-endian bytes
#[test]
fn test_encode_u32_little_endian() {
    let value = 0x12345678u32;
    let bytes = encode_u32(value, Endianness::LittleEndian);

    assert_eq!(bytes, [0x78, 0x56, 0x34, 0x12]);
}

/// Test encoding u32 to big-endian bytes
#[test]
fn test_encode_u32_big_endian() {
    let value = 0x12345678u32;
    let bytes = encode_u32(value, Endianness::BigEndian);

    assert_eq!(bytes, [0x12, 0x34, 0x56, 0x78]);
}

// ============================================================================
// u64 Encoding Tests
// ============================================================================

/// Test encoding u64 to little-endian bytes
#[test]
fn test_encode_u64_little_endian() {
    let value = 0x123456789ABCDEF0u64;
    let bytes = encode_u64(value, Endianness::LittleEndian);

    assert_eq!(bytes, [0xF0, 0xDE, 0xBC, 0x9A, 0x78, 0x56, 0x34, 0x12]);
}

/// Test encoding u64 to big-endian bytes
#[test]
fn test_encode_u64_big_endian() {
    let value = 0x123456789ABCDEF0u64;
    let bytes = encode_u64(value, Endianness::BigEndian);

    assert_eq!(bytes, [0x12, 0x34, 0x56, 0x78, 0x9A, 0xBC, 0xDE, 0xF0]);
}

// ============================================================================
// Round-Trip Tests
// ============================================================================

/// Test round-trip conversion for u32 (parse then encode)
#[test]
fn test_u32_round_trip() {
    let original = 0x12345678u32;

    // Little-endian round-trip
    let le_bytes = encode_u32(original, Endianness::LittleEndian);
    let le_parsed = parse_u32(&le_bytes, Endianness::LittleEndian).unwrap();
    assert_eq!(le_parsed, original);

    // Big-endian round-trip
    let be_bytes = encode_u32(original, Endianness::BigEndian);
    let be_parsed = parse_u32(&be_bytes, Endianness::BigEndian).unwrap();
    assert_eq!(be_parsed, original);
}

/// Test round-trip conversion for u64 (parse then encode)
#[test]
fn test_u64_round_trip() {
    let original = 0x123456789ABCDEF0u64;

    // Little-endian round-trip
    let le_bytes = encode_u64(original, Endianness::LittleEndian);
    let le_parsed = parse_u64(&le_bytes, Endianness::LittleEndian).unwrap();
    assert_eq!(le_parsed, original);

    // Big-endian round-trip
    let be_bytes = encode_u64(original, Endianness::BigEndian);
    let be_parsed = parse_u64(&be_bytes, Endianness::BigEndian).unwrap();
    assert_eq!(be_parsed, original);
}

/// Test conversion for various data types (u32, u64, addresses)
#[test]
fn test_conversion_various_types() {
    // Test u32
    let u32_val = 0xAABBCCDDu32;
    let u32_converted = convert_u32(u32_val, Endianness::LittleEndian, Endianness::BigEndian);
    assert_eq!(u32_converted, 0xDDCCBBAAu32);

    // Test u64
    let u64_val = 0x1122334455667788u64;
    let u64_converted = convert_u64(u64_val, Endianness::LittleEndian, Endianness::BigEndian);
    assert_eq!(u64_converted, 0x8877665544332211u64);

    // Test address
    let addr_bytes = [
        0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F,
        0x10, 0x11, 0x12, 0x13, 0x14,
    ];
    let addr_converted =
        convert_address(&addr_bytes, Endianness::LittleEndian, Endianness::BigEndian).unwrap();
    assert_eq!(addr_converted[0], 0x14);
    assert_eq!(addr_converted[19], 0x01);
}

/// Test round-trip conversion correctness (convert back and forth)
#[test]
fn test_round_trip_correctness() {
    let original_u32 = 0x12345678u32;
    let converted_once = convert_u32(
        original_u32,
        Endianness::LittleEndian,
        Endianness::BigEndian,
    );
    let converted_twice = convert_u32(
        converted_once,
        Endianness::BigEndian,
        Endianness::LittleEndian,
    );
    assert_eq!(converted_twice, original_u32);

    let original_u64 = 0x123456789ABCDEF0u64;
    let converted_once_64 = convert_u64(
        original_u64,
        Endianness::LittleEndian,
        Endianness::BigEndian,
    );
    let converted_twice_64 = convert_u64(
        converted_once_64,
        Endianness::BigEndian,
        Endianness::LittleEndian,
    );
    assert_eq!(converted_twice_64, original_u64);
}
