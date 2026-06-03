use nimiq_primitives::coin::Coin;
use nimiq_transaction::{
    account::htlc_contract::{AnyHash, AnyHash32},
    bridge_contract::{
        AddressFormat, ChainConfig, Endianness, IncomingTransaction, RecipientData, ValidationOp,
        ValidationProgram,
    },
};

/// Helper function to create a test incoming transaction
fn create_test_incoming_tx(amount: u64, nonce: u64, validity_height: u32) -> IncomingTransaction {
    let recipient_data = RecipientData {
        target_address: vec![0xAB; 20],
        target_nonce: nonce,
    };

    IncomingTransaction::new(
        recipient_data,
        Coin::from_u64_unchecked(amount),
        validity_height,
    )
    .unwrap()
}

#[test]
fn test_validation_op_push_const() {
    let program = ValidationProgram::new(vec![
        ValidationOp::PushConst(42),
        ValidationOp::PushConst(100),
    ]);

    let incoming_tx = create_test_incoming_tx(1000, 1, 100);
    let burn_tx_data = vec![0u8; 100];

    // Should succeed - just pushes constants
    assert!(program.execute(&burn_tx_data, &incoming_tx).is_ok());
}

#[test]
fn test_validation_op_push_expected_amount() {
    let program = ValidationProgram::new(vec![
        ValidationOp::PushExpectedAmount,
        ValidationOp::PushConst(1000),
        ValidationOp::Eq,
        ValidationOp::Assert,
    ]);

    let incoming_tx = create_test_incoming_tx(1000, 1, 100);
    let burn_tx_data = vec![0u8; 100];

    // Should succeed - amount matches
    assert!(program.execute(&burn_tx_data, &incoming_tx).is_ok());
}

#[test]
fn test_validation_op_push_expected_amount_mismatch() {
    let program = ValidationProgram::new(vec![
        ValidationOp::PushExpectedAmount,
        ValidationOp::PushConst(2000), // Different amount
        ValidationOp::Eq,
        ValidationOp::Assert,
    ]);

    let incoming_tx = create_test_incoming_tx(1000, 1, 100);
    let burn_tx_data = vec![0u8; 100];

    // Should fail - amount doesn't match
    assert!(program.execute(&burn_tx_data, &incoming_tx).is_err());
}

#[test]
fn test_validation_op_load_u64_little_endian() {
    // Create burn transaction data with a u64 value at offset 0
    let mut burn_tx_data = vec![0u8; 100];
    let value: u64 = 1234567890;
    burn_tx_data[0..8].copy_from_slice(&value.to_le_bytes());

    let program = ValidationProgram::new(vec![
        ValidationOp::PushConst(0), // Offset
        ValidationOp::LoadU64(Endianness::LittleEndian),
        ValidationOp::PushConst(1234567890),
        ValidationOp::Eq,
        ValidationOp::Assert,
    ]);

    let incoming_tx = create_test_incoming_tx(1000, 1, 100);

    // Should succeed - value matches
    assert!(program.execute(&burn_tx_data, &incoming_tx).is_ok());
}

#[test]
fn test_validation_op_load_u64_big_endian() {
    // Create burn transaction data with a u64 value at offset 0
    let mut burn_tx_data = vec![0u8; 100];
    let value: u64 = 1234567890;
    burn_tx_data[0..8].copy_from_slice(&value.to_be_bytes());

    let program = ValidationProgram::new(vec![
        ValidationOp::PushConst(0), // Offset
        ValidationOp::LoadU64(Endianness::BigEndian),
        ValidationOp::PushConst(1234567890),
        ValidationOp::Eq,
        ValidationOp::Assert,
    ]);

    let incoming_tx = create_test_incoming_tx(1000, 1, 100);

    // Should succeed - value matches
    assert!(program.execute(&burn_tx_data, &incoming_tx).is_ok());
}

#[test]
fn test_validation_op_load_address() {
    // Create burn transaction data with an address at offset 10
    let mut burn_tx_data = vec![0u8; 100];
    let address_bytes = [0xAB; 20];
    burn_tx_data[10..30].copy_from_slice(&address_bytes);

    let program = ValidationProgram::new(vec![
        ValidationOp::PushConst(10), // Offset
        ValidationOp::LoadAddress,
        ValidationOp::PushExpectedAddress,
        ValidationOp::Eq,
        ValidationOp::Assert,
    ]);

    let incoming_tx = create_test_incoming_tx(1000, 1, 100);

    // Should succeed - address matches
    assert!(program.execute(&burn_tx_data, &incoming_tx).is_ok());
}

#[test]
fn test_validation_op_arithmetic_add() {
    let program = ValidationProgram::new(vec![
        ValidationOp::PushConst(10),
        ValidationOp::PushConst(20),
        ValidationOp::Add,
        ValidationOp::PushConst(30),
        ValidationOp::Eq,
        ValidationOp::Assert,
    ]);

    let incoming_tx = create_test_incoming_tx(1000, 1, 100);
    let burn_tx_data = vec![0u8; 100];

    // Should succeed - 10 + 20 = 30
    assert!(program.execute(&burn_tx_data, &incoming_tx).is_ok());
}

#[test]
fn test_validation_op_arithmetic_sub() {
    let program = ValidationProgram::new(vec![
        ValidationOp::PushConst(50),
        ValidationOp::PushConst(20),
        ValidationOp::Sub,
        ValidationOp::PushConst(30),
        ValidationOp::Eq,
        ValidationOp::Assert,
    ]);

    let incoming_tx = create_test_incoming_tx(1000, 1, 100);
    let burn_tx_data = vec![0u8; 100];

    // Should succeed - 50 - 20 = 30
    assert!(program.execute(&burn_tx_data, &incoming_tx).is_ok());
}

#[test]
fn test_validation_op_arithmetic_mul() {
    let program = ValidationProgram::new(vec![
        ValidationOp::PushConst(10),
        ValidationOp::PushConst(20),
        ValidationOp::Mul,
        ValidationOp::PushConst(200),
        ValidationOp::Eq,
        ValidationOp::Assert,
    ]);

    let incoming_tx = create_test_incoming_tx(1000, 1, 100);
    let burn_tx_data = vec![0u8; 100];

    // Should succeed - 10 * 20 = 200
    assert!(program.execute(&burn_tx_data, &incoming_tx).is_ok());
}

#[test]
fn test_validation_op_arithmetic_div() {
    let program = ValidationProgram::new(vec![
        ValidationOp::PushConst(100),
        ValidationOp::PushConst(20),
        ValidationOp::Div,
        ValidationOp::PushConst(5),
        ValidationOp::Eq,
        ValidationOp::Assert,
    ]);

    let incoming_tx = create_test_incoming_tx(1000, 1, 100);
    let burn_tx_data = vec![0u8; 100];

    // Should succeed - 100 / 20 = 5
    assert!(program.execute(&burn_tx_data, &incoming_tx).is_ok());
}

#[test]
fn test_validation_op_comparison_lt() {
    let program = ValidationProgram::new(vec![
        ValidationOp::PushConst(10),
        ValidationOp::PushConst(20),
        ValidationOp::Lt,
        ValidationOp::PushConst(1), // True
        ValidationOp::Eq,
        ValidationOp::Assert,
    ]);

    let incoming_tx = create_test_incoming_tx(1000, 1, 100);
    let burn_tx_data = vec![0u8; 100];

    // Should succeed - 10 < 20 is true
    assert!(program.execute(&burn_tx_data, &incoming_tx).is_ok());
}

#[test]
fn test_validation_op_comparison_gt() {
    let program = ValidationProgram::new(vec![
        ValidationOp::PushConst(30),
        ValidationOp::PushConst(20),
        ValidationOp::Gt,
        ValidationOp::PushConst(1), // True
        ValidationOp::Eq,
        ValidationOp::Assert,
    ]);

    let incoming_tx = create_test_incoming_tx(1000, 1, 100);
    let burn_tx_data = vec![0u8; 100];

    // Should succeed - 30 > 20 is true
    assert!(program.execute(&burn_tx_data, &incoming_tx).is_ok());
}

#[test]
fn test_validation_op_stack_dup() {
    let program = ValidationProgram::new(vec![
        ValidationOp::PushConst(42),
        ValidationOp::Dup,
        ValidationOp::Eq,
        ValidationOp::Assert,
    ]);

    let incoming_tx = create_test_incoming_tx(1000, 1, 100);
    let burn_tx_data = vec![0u8; 100];

    // Should succeed - duplicated value equals itself
    assert!(program.execute(&burn_tx_data, &incoming_tx).is_ok());
}

#[test]
fn test_validation_op_stack_swap() {
    let program = ValidationProgram::new(vec![
        ValidationOp::PushConst(10),
        ValidationOp::PushConst(20),
        ValidationOp::Swap,
        ValidationOp::PushConst(10),
        ValidationOp::Eq,
        ValidationOp::Assert,
    ]);

    let incoming_tx = create_test_incoming_tx(1000, 1, 100);
    let burn_tx_data = vec![0u8; 100];

    // Should succeed - after swap, top is 10
    assert!(program.execute(&burn_tx_data, &incoming_tx).is_ok());
}

/// Test a complete Ethereum-style validation program
#[test]
fn test_ethereum_validation_program() {
    // Simulate Ethereum burn transaction data:
    // - Offset 0-31: Amount (EVM format, 32 bytes)
    // - Offset 32-51: Address (20 bytes)
    // - Offset 52-59: Nonce (8 bytes, big-endian)
    // - Offset 60-63: Validity height (4 bytes, big-endian)

    let mut burn_tx_data = vec![0u8; 100];

    // Amount: 1000 in EVM format (32 bytes, big-endian)
    let mut amount_bytes = [0u8; 32];
    amount_bytes[24..32].copy_from_slice(&1000u64.to_be_bytes());
    burn_tx_data[0..32].copy_from_slice(&amount_bytes);

    // Address: [0xAB; 20]
    burn_tx_data[32..52].copy_from_slice(&[0xAB; 20]);

    // Nonce: 1 (big-endian)
    burn_tx_data[52..60].copy_from_slice(&1u64.to_be_bytes());

    // Validity height: 100 (big-endian)
    burn_tx_data[60..64].copy_from_slice(&100u32.to_be_bytes());

    let program = ValidationProgram::new(vec![
        // Validate amount
        ValidationOp::PushConst(0),
        ValidationOp::LoadEvmU64,
        ValidationOp::PushExpectedAmount,
        ValidationOp::Eq,
        ValidationOp::Assert,
        // Validate address
        ValidationOp::PushConst(32),
        ValidationOp::LoadAddress,
        ValidationOp::PushExpectedAddress,
        ValidationOp::Eq,
        ValidationOp::Assert,
        // Validate nonce
        ValidationOp::PushConst(52),
        ValidationOp::LoadU64(Endianness::BigEndian),
        ValidationOp::PushExpectedNonce,
        ValidationOp::Eq,
        ValidationOp::Assert,
        // Validate validity height
        ValidationOp::PushConst(60),
        ValidationOp::LoadU32(Endianness::BigEndian),
        ValidationOp::PushExpectedValidityHeight,
        ValidationOp::Eq,
        ValidationOp::Assert,
    ]);

    let incoming_tx = create_test_incoming_tx(1000, 1, 100);

    // Should succeed - all fields match
    assert!(program.execute(&burn_tx_data, &incoming_tx).is_ok());
}

/// Test a complete Bitcoin-style validation program
#[test]
fn test_bitcoin_validation_program() {
    // Simulate Bitcoin OP_RETURN data:
    // - Offset 0-7: Amount (8 bytes, big-endian)
    // - Offset 8-27: Address (20 bytes)
    // - Offset 28-35: Nonce (8 bytes, big-endian)
    // - Offset 36-39: Validity height (4 bytes, big-endian)

    let mut burn_tx_data = vec![0u8; 100];

    // Amount: 1000 (big-endian)
    burn_tx_data[0..8].copy_from_slice(&1000u64.to_be_bytes());

    // Address: [0xAB; 20]
    burn_tx_data[8..28].copy_from_slice(&[0xAB; 20]);

    // Nonce: 1 (big-endian)
    burn_tx_data[28..36].copy_from_slice(&1u64.to_be_bytes());

    // Validity height: 100 (big-endian)
    burn_tx_data[36..40].copy_from_slice(&100u32.to_be_bytes());

    let program = ValidationProgram::new(vec![
        // Validate amount
        ValidationOp::PushConst(0),
        ValidationOp::LoadU64(Endianness::BigEndian),
        ValidationOp::PushExpectedAmount,
        ValidationOp::Eq,
        ValidationOp::Assert,
        // Validate address
        ValidationOp::PushConst(8),
        ValidationOp::LoadAddress,
        ValidationOp::PushExpectedAddress,
        ValidationOp::Eq,
        ValidationOp::Assert,
        // Validate nonce
        ValidationOp::PushConst(28),
        ValidationOp::LoadU64(Endianness::BigEndian),
        ValidationOp::PushExpectedNonce,
        ValidationOp::Eq,
        ValidationOp::Assert,
        // Validate validity height
        ValidationOp::PushConst(36),
        ValidationOp::LoadU32(Endianness::BigEndian),
        ValidationOp::PushExpectedValidityHeight,
        ValidationOp::Eq,
        ValidationOp::Assert,
    ]);

    let incoming_tx = create_test_incoming_tx(1000, 1, 100);

    // Should succeed - all fields match
    assert!(program.execute(&burn_tx_data, &incoming_tx).is_ok());
}

/// Test validation with fee calculation
#[test]
fn test_validation_with_fee_calculation() {
    // Burn amount should equal expected amount + 1% fee
    let mut burn_tx_data = vec![0u8; 100];

    // Burn amount: 1010 (1000 + 1% = 1000 + 10)
    burn_tx_data[0..8].copy_from_slice(&1010u64.to_le_bytes());

    let program = ValidationProgram::new(vec![
        // Load burn amount
        ValidationOp::PushConst(0),
        ValidationOp::LoadU64(Endianness::LittleEndian),
        // Calculate expected total (amount + 1% fee)
        ValidationOp::PushExpectedAmount, // 1000
        ValidationOp::Dup,                // 1000, 1000
        ValidationOp::PushConst(100),     // 1000, 1000, 100
        ValidationOp::Div,                // 1000, 10 (1% of 1000)
        ValidationOp::Add,                // 1010
        // Compare
        ValidationOp::Eq,
        ValidationOp::Assert,
    ]);

    let incoming_tx = create_test_incoming_tx(1000, 1, 100);

    // Should succeed - burn amount equals amount + 1% fee
    assert!(program.execute(&burn_tx_data, &incoming_tx).is_ok());
}

#[test]
fn test_empty_validation_program() {
    let program = ValidationProgram::empty();

    let incoming_tx = create_test_incoming_tx(1000, 1, 100);
    let burn_tx_data = vec![0u8; 100];

    // Should succeed - no validation
    assert!(program.execute(&burn_tx_data, &incoming_tx).is_ok());
}

#[test]
fn test_chain_config_with_validation_program() {
    let validation_program = ValidationProgram::new(vec![
        ValidationOp::PushExpectedAmount,
        ValidationOp::PushConst(1000),
        ValidationOp::Eq,
        ValidationOp::Assert,
    ]);

    let chain_config = ChainConfig {
        chain_id: 1,
        hash_function: AnyHash::Blake2b(AnyHash32::default()),
        address_format: AddressFormat::Ethereum,
        endianness: Endianness::BigEndian,
        block_time: std::time::Duration::from_secs(12),
        validation_program,
        max_proof_depth: 64,
    };

    // Verify the validation program is stored correctly
    assert_eq!(chain_config.validation_program.operations.len(), 4);
}
