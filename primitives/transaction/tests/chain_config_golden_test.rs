//! Golden tests for the Polygon Amoy `ChainConfig`.
//!
//! A bridge is created with its `ChainConfig` serialized into an opaque hex blob. The validation
//! program embedded in that blob is what turns an EVM `TokensBurned` event into the amount,
//! address and nonce that consensus pays out against, and consensus cannot tell a correct program
//! from a subtly wrong one: a mis-scaled constant there mis-pays real money in a way no other
//! check catches.
//!
//! These tests pin the blob byte-for-byte, decode every field, and assert the exact values the
//! program extracts from realistic burn payloads, so a change to either the blob or the encoding
//! fails here instead of on-chain.

use nimiq_keys::Address;
use nimiq_primitives::coin::Coin;
use nimiq_serde::{Deserialize, Serialize};
use nimiq_transaction::{
    account::htlc_contract::{AnyHash, AnyHash32},
    bridge_contract::{
        AddressFormat, AnyMerkleProof, BridgeError, ChainConfig, Endianness, OutgoingTransaction,
        StackValue, ValidationOp, ValidationProgram,
    },
};
use nimiq_utils::merkle::MerklePath;

/// The serialized `ChainConfig` for Polygon Amoy. Redefining it means every bridge instance
/// deployed with the old blob has to be redeployed, and whatever emits it has to be updated to
/// match.
const POLYGON_AMOY_CHAIN_CONFIG: &str = "82f104050000000000000000000000000000000000000000000000000000000000000000010102001000340680c0caf384a3021c06616d6f756e740000041c0e7461726765745f616464726573730014051c0c7461726765745f6e6f6e636500140500ffffffff0f141c116275726e5f626c6f636b5f6865696768740082f1041c0f7461726765745f636861696e5f696420";

const AMOY_CHAIN_ID: u32 = 80002;

/// Layout of the burn payload a relayer derives from a `TokensBurned` event: the indexed target
/// followed by the two non-indexed words, 84 bytes, tightly packed and with no header.
const TARGET_OFFSET: u64 = 0; // 20 raw address bytes
const NONCE_OFFSET: u64 = 20; // 32-byte big-endian word
const AMOUNT_OFFSET: u64 = 52; // 32-byte big-endian word
const BURN_PAYLOAD_LEN: usize = 84;

/// wNIM carries 18 decimals and NIM has 5 (luna), so one luna is 10^13 wei.
const WEI_PER_LUNA: u64 = 10_000_000_000_000;

/// One NIM in luna, for readable vectors.
const LUNA_PER_NIM: u64 = 100_000;

fn amoy_chain_config() -> ChainConfig {
    ChainConfig::deserialize_from_vec(&hex::decode(POLYGON_AMOY_CHAIN_CONFIG).unwrap())
        .expect("shipped ChainConfig blob must deserialize")
}

/// Encodes a 32-byte big-endian EVM word.
fn evm_word(value: u128) -> [u8; 32] {
    let mut word = [0u8; 32];
    word[16..32].copy_from_slice(&value.to_be_bytes());
    word
}

/// Builds the 84-byte burn payload for a `TokensBurned(to, nimiq_nonce, amount)` event.
fn burn_payload(target: [u8; 20], nimiq_nonce: u128, amount_wei: u128) -> Vec<u8> {
    let mut payload = Vec::with_capacity(BURN_PAYLOAD_LEN);
    payload.extend_from_slice(&target);
    payload.extend_from_slice(&evm_word(nimiq_nonce));
    payload.extend_from_slice(&evm_word(amount_wei));
    assert_eq!(payload.len(), BURN_PAYLOAD_LEN);
    payload
}

/// The program the Amoy blob is expected to carry, written out independently of the blob so a
/// changed opcode shows up as a diff rather than as a silently-accepted new program.
fn expected_amoy_program() -> ValidationProgram {
    ValidationProgram::new(vec![
        // amount: the 32-byte word at offset 52, divided down to luna at full 256-bit width
        // before narrowing. Reading it with `LoadEvmU64` and dividing afterwards would require
        // the *wei* figure to fit a u64, capping a burn at about 18.44 NIM.
        ValidationOp::PushConst(AMOUNT_OFFSET),
        ValidationOp::LoadEvmU64Scaled(WEI_PER_LUNA),
        ValidationOp::Store("amount".to_string()),
        // target_address: the 20 raw bytes at offset 0.
        ValidationOp::PushConst(TARGET_OFFSET),
        ValidationOp::LoadAddress,
        ValidationOp::Store("target_address".to_string()),
        // target_nonce: the 32-byte word at offset 20.
        ValidationOp::PushConst(NONCE_OFFSET),
        ValidationOp::LoadEvmU64,
        ValidationOp::Store("target_nonce".to_string()),
        // burn_block_height: re-reads the nonce word and combines it with u32::MAX. See
        // `amoy_burn_block_height_is_a_logical_flag_not_a_masked_nonce` for what this yields.
        ValidationOp::PushConst(NONCE_OFFSET),
        ValidationOp::LoadEvmU64,
        ValidationOp::PushConst(u32::MAX as u64),
        ValidationOp::And,
        ValidationOp::Store("burn_block_height".to_string()),
        // target_chain_id: the constant the bridge matches against its own source_chain_id.
        ValidationOp::PushConst(AMOY_CHAIN_ID as u64),
        ValidationOp::Store("target_chain_id".to_string()),
    ])
}

fn expected_amoy_chain_config() -> ChainConfig {
    ChainConfig {
        chain_id: AMOY_CHAIN_ID,
        hash_function: AnyHash::Keccak256(AnyHash32::from([0u8; 32])),
        address_format: AddressFormat::Ethereum,
        endianness: Endianness::BigEndian,
        block_time: std::time::Duration::from_secs(2),
        validation_program: expected_amoy_program(),
        max_proof_depth: 32,
    }
}

/// Wraps a burn payload in an `OutgoingTransaction` so `parse_burn_data` can be exercised. The
/// proof itself is irrelevant here: these tests are about extraction, not inclusion.
fn outgoing_tx(burn_payload: Vec<u8>) -> OutgoingTransaction {
    OutgoingTransaction {
        burn_transaction_data: burn_payload,
        merkle_proof: AnyMerkleProof::Keccak256Path(MerklePath::empty()),
        oracle_state_index: 0,
    }
}

fn extract(config: &ChainConfig, payload: &[u8], name: &str) -> StackValue {
    config
        .validation_program
        .extract_only(payload)
        .unwrap_or_else(|e| panic!("extraction failed: {e:?}"))
        .extracted_values
        .get(name)
        .unwrap_or_else(|| panic!("program did not store `{name}`"))
        .clone()
}

// ---------------------------------------------------------------------------------------------
// 1. Golden decode: every field of the blob.
// ---------------------------------------------------------------------------------------------

#[test]
fn amoy_chain_config_decodes_to_the_expected_fields() {
    let config = amoy_chain_config();

    assert_eq!(config.chain_id, AMOY_CHAIN_ID, "chain id must be Amoy");
    assert_eq!(
        config.hash_function,
        AnyHash::Keccak256(AnyHash32::from([0u8; 32])),
        "Amoy proofs are Keccak256; the hash bytes are an unused algorithm marker"
    );
    assert_eq!(config.address_format, AddressFormat::Ethereum);
    assert_eq!(
        config.endianness,
        Endianness::BigEndian,
        "EVM words are big-endian"
    );
    assert_eq!(config.block_time, std::time::Duration::from_secs(2));
    assert_eq!(
        config.max_proof_depth, 32,
        "bounds proof verification work to trees of 2^32 leaves"
    );
    assert_eq!(
        config.validation_program.operations.len(),
        16,
        "the Amoy program is 16 opcodes"
    );
    assert_eq!(
        config.validation_program,
        expected_amoy_program(),
        "opcode sequence drifted from the reviewed program"
    );
}

/// The program must be extraction-only. `Assert` and `PushExpected*` need an `IncomingTransaction`
/// that the release path does not have, so a program containing them locks the bridge's funds --
/// which is why bridge creation rejects them outright.
#[test]
fn amoy_chain_config_program_is_extraction_only() {
    for op in &amoy_chain_config().validation_program.operations {
        assert!(
            !matches!(
                op,
                ValidationOp::Assert
                    | ValidationOp::PushExpectedAmount
                    | ValidationOp::PushExpectedAddress
                    | ValidationOp::PushExpectedNonce
                    | ValidationOp::PushExpectedValidityHeight
            ),
            "fund-locking opcode {op:?} in the shipped program"
        );
    }
}

// ---------------------------------------------------------------------------------------------
// 2. Round-trip: the pinned blob == a fresh serialization.
// ---------------------------------------------------------------------------------------------

#[test]
fn amoy_chain_config_reserializes_to_the_pinned_blob() {
    // Built here and serialized, then compared against the pinned blob. Catches drift in either
    // direction: a changed constant, or a changed encoding.
    assert_eq!(
        hex::encode(expected_amoy_chain_config().serialize_to_vec()),
        POLYGON_AMOY_CHAIN_CONFIG,
        "the serialization no longer reproduces the pinned blob"
    );

    // And the blob survives a decode/encode cycle byte-for-byte.
    assert_eq!(
        hex::encode(amoy_chain_config().serialize_to_vec()),
        POLYGON_AMOY_CHAIN_CONFIG
    );
}

// ---------------------------------------------------------------------------------------------
// 3. Golden vectors: what the program extracts from real burn events.
// ---------------------------------------------------------------------------------------------

#[test]
fn amoy_program_extracts_expected_values_from_burn_events() {
    let config = amoy_chain_config();

    // (description, target, nonce, amount_wei, expected luna)
    let vectors: [(&str, [u8; 20], u128, u128, u64); 6] = [
        (
            "1 NIM",
            [0x11; 20],
            1,
            1_000_000_000_000_000_000,
            LUNA_PER_NIM,
        ),
        ("1 luna (dust)", [0x22; 20], 42, WEI_PER_LUNA as u128, 1),
        (
            "12.5 NIM",
            [0xAB; 20],
            7,
            12_500_000_000_000_000_000,
            12 * LUNA_PER_NIM + 50_000,
        ),
        (
            // 1,000 NIM is 10^21 wei, some 54 times more than a u64 holds. Dividing at full
            // width is what makes an ordinary transfer expressible at all.
            "1,000 NIM",
            [0xFF; 20],
            u32::MAX as u128,
            1_000 * LUNA_PER_NIM as u128 * WEI_PER_LUNA as u128,
            1_000 * LUNA_PER_NIM,
        ),
        (
            // The largest amount a release can carry at all: above this the value is no longer a
            // valid `Coin`, so `Coin::MAX` is the real ceiling rather than the EVM word.
            "the Coin ceiling",
            [0xCD; 20],
            11,
            Coin::MAX_SAFE_VALUE as u128 * WEI_PER_LUNA as u128,
            Coin::MAX_SAFE_VALUE,
        ),
        (
            "nonce above u32, still a valid u64",
            [0x01; 20],
            u64::MAX as u128,
            1_000_000_000_000_000_000,
            LUNA_PER_NIM,
        ),
    ];

    for (description, target, nonce, amount_wei, expected_luna) in vectors {
        let payload = burn_payload(target, nonce, amount_wei);

        assert_eq!(
            extract(&config, &payload, "amount"),
            StackValue::U64(expected_luna),
            "{description}: wrong luna amount"
        );
        assert_eq!(
            extract(&config, &payload, "target_address"),
            StackValue::Bytes(target.to_vec()),
            "{description}: wrong target address"
        );
        assert_eq!(
            extract(&config, &payload, "target_nonce"),
            StackValue::U64(nonce as u64),
            "{description}: wrong nonce"
        );
        assert_eq!(
            extract(&config, &payload, "target_chain_id"),
            StackValue::U64(AMOY_CHAIN_ID as u64),
            "{description}: wrong chain id"
        );

        // The same values, seen through the struct consensus actually pays out against.
        let parsed = outgoing_tx(payload).parse_burn_data(&config).unwrap();
        assert_eq!(
            parsed.amount,
            Coin::from_u64_unchecked(expected_luna),
            "{description}: parse_burn_data amount"
        );
        assert_eq!(
            parsed.target_address,
            Address::from(&target[..]),
            "{description}: parse_burn_data address"
        );
        assert_eq!(
            parsed.target_nonce, nonce as u64,
            "{description}: parse_burn_data nonce"
        );
        assert_eq!(
            parsed.target_chain_id, AMOY_CHAIN_ID,
            "{description}: parse_burn_data chain id"
        );
    }
}

/// Consensus divides wei by 10^13 and truncates. Whole-luna granularity is enforced only by the
/// EVM contract (`amount % CONVERSION_FACTOR == 0` in `EvmBridge.burn`), so if that check were
/// ever removed the sub-luna remainder would be burned on Polygon and silently dropped here.
#[test]
fn amoy_program_truncates_sub_luna_remainders() {
    let config = amoy_chain_config();
    let payload = burn_payload(
        [0x33; 20],
        1,
        1_000_000_000_000_000_000 + WEI_PER_LUNA as u128 - 1,
    );

    assert_eq!(
        extract(&config, &payload, "amount"),
        StackValue::U64(LUNA_PER_NIM),
        "one luna minus one wei of remainder must be truncated, not rounded up"
    );
}

/// `And` is a logical operator in this VM, not a bitwise mask, so `nonce AND u32::MAX` collapses
/// to 1 for every non-zero nonce rather than yielding the low 32 bits. `burn_block_height` is
/// therefore a constant 1 on Amoy, not a height. Nothing in the release path reads it beyond a
/// non-zero check, so this is currently harmless -- but it is pinned here because a program that
/// parses but extracts the wrong field is exactly what these golden tests exist to surface, and
/// because a future bitwise `And` would silently change what this deployed program computes.
#[test]
fn amoy_burn_block_height_is_a_logical_flag_not_a_masked_nonce() {
    let config = amoy_chain_config();

    for nonce in [1u128, 2, u32::MAX as u128, 1u128 << 40, u64::MAX as u128] {
        let payload = burn_payload([0x44; 20], nonce, 1_000_000_000_000_000_000);
        assert_eq!(
            extract(&config, &payload, "burn_block_height"),
            StackValue::U64(1),
            "nonce {nonce} should collapse to the logical flag 1, not to its low 32 bits"
        );
    }

    // A zero nonce makes the flag zero, and `ParsedBurnData` rejects both.
    let payload = burn_payload([0x44; 20], 0, 1_000_000_000_000_000_000);
    assert_eq!(
        extract(&config, &payload, "burn_block_height"),
        StackValue::U64(0)
    );
    assert!(matches!(
        outgoing_tx(payload).parse_burn_data(&config),
        Err(BridgeError::InvalidRecipientData)
    ));
}

// ---------------------------------------------------------------------------------------------
// 4. Boundary: the per-burn ceiling this program imposes.
// ---------------------------------------------------------------------------------------------

/// The amount is divided at full 256-bit width before it is narrowed, so what has to fit a u64 is
/// the *luna* figure and not the wei one. Extraction therefore tops out at `u64::MAX` luna, and
/// `Coin` is stricter still, so `Coin` is the limit a user can actually meet. Both are pinned:
/// they are properties of this program, and neither must move silently.
///
/// Reading the word with `LoadEvmU64` and dividing afterwards would put the ceiling at `u64::MAX`
/// *wei* — about 18.44674 NIM, below any ordinary transfer.
#[test]
fn amoy_program_ceiling_is_the_coin_limit_rather_than_the_evm_word() {
    let config = amoy_chain_config();

    // The entire supply is expressible, which is the point of the width.
    let total_supply_luna = 21_000_000_000u64 * LUNA_PER_NIM;
    assert!(total_supply_luna < Coin::MAX_SAFE_VALUE);
    let payload = burn_payload(
        [0x55; 20],
        1,
        total_supply_luna as u128 * WEI_PER_LUNA as u128,
    );
    assert_eq!(
        extract(&config, &payload, "amount"),
        StackValue::U64(total_supply_luna),
    );

    // Extraction tops out at u64::MAX luna exactly...
    let payload = burn_payload([0x55; 20], 1, u64::MAX as u128 * WEI_PER_LUNA as u128);
    assert_eq!(
        extract(&config, &payload, "amount"),
        StackValue::U64(u64::MAX)
    );
    // ...and a remainder below one luna does not push it over.
    let payload = burn_payload(
        [0x55; 20],
        1,
        u64::MAX as u128 * WEI_PER_LUNA as u128 + WEI_PER_LUNA as u128 - 1,
    );
    assert_eq!(
        extract(&config, &payload, "amount"),
        StackValue::U64(u64::MAX)
    );

    // One luna more and the quotient no longer fits: refused, never truncated.
    for amount_wei in [(u64::MAX as u128 + 1) * WEI_PER_LUNA as u128, u128::MAX] {
        let payload = burn_payload([0x55; 20], 1, amount_wei);
        assert!(
            matches!(
                config.validation_program.extract_only(&payload),
                Err(BridgeError::InvalidAmount)
            ),
            "amount {amount_wei} wei must be rejected, not truncated"
        );
    }

    // `Coin` binds first, so that is the ceiling a release can actually reach.
    let above_coin = (Coin::MAX_SAFE_VALUE as u128 + 1) * WEI_PER_LUNA as u128;
    let payload = burn_payload([0x55; 20], 1, above_coin);
    assert_eq!(
        extract(&config, &payload, "amount"),
        StackValue::U64(Coin::MAX_SAFE_VALUE + 1),
        "extraction still reads it..."
    );
    assert!(
        matches!(
            outgoing_tx(payload).parse_burn_data(&config),
            Err(BridgeError::InvalidAmount)
        ),
        "...but it cannot become a Coin, so the release is refused"
    );
    // Exactly at the Coin limit still parses.
    let payload = burn_payload(
        [0x55; 20],
        1,
        Coin::MAX_SAFE_VALUE as u128 * WEI_PER_LUNA as u128,
    );
    assert_eq!(
        outgoing_tx(payload)
            .parse_burn_data(&config)
            .unwrap()
            .amount,
        Coin::from_u64_unchecked(Coin::MAX_SAFE_VALUE),
    );
}

#[test]
fn amoy_program_rejects_nonces_above_the_u64_word_ceiling() {
    let config = amoy_chain_config();
    let payload = burn_payload([0x66; 20], u64::MAX as u128 + 1, 1_000_000_000_000_000_000);

    assert!(matches!(
        config.validation_program.extract_only(&payload),
        Err(BridgeError::InvalidAmount)
    ));
}

/// The program indexes fixed offsets into an 84-byte payload. A short payload must produce a typed
/// error, never an out-of-bounds read.
#[test]
fn amoy_program_rejects_payloads_shorter_than_the_event_layout() {
    let config = amoy_chain_config();
    let full = burn_payload([0x77; 20], 1, 1_000_000_000_000_000_000);

    for len in [0, 1, 20, 52, 83] {
        assert!(
            matches!(
                config.validation_program.extract_only(&full[..len]),
                Err(BridgeError::InvalidDataLength)
            ),
            "{len}-byte payload must be rejected as too short"
        );
    }

    assert!(config.validation_program.extract_only(&full).is_ok());
}
