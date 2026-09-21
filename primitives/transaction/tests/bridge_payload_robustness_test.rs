//! Robustness of burn-payload parsing against malformed input.
//!
//! A release hands the validation program a payload that anyone can submit, and the program
//! itself is chosen by whoever created the bridge. Consensus therefore has to survive both being
//! hostile: every short, empty or garbled payload — and every program — must end in a typed
//! rejection, never a panic and never unbounded work. The per-opcode bounds checks are covered
//! elsewhere; these tests sweep whole programs across every truncation point, then randomize the
//! payload, and finally randomize the program too.

mod amoy;

use std::{sync::mpsc, thread, time::Duration};

use amoy::amoy_chain_config;
use nimiq_primitives::coin::Coin;
use nimiq_test_utils::test_rng;
use nimiq_transaction::{
    account::htlc_contract::{AnyHash, AnyHash32},
    bridge_contract::{
        AddressFormat, AnyMerkleProof, BridgeError, ChainConfig, Endianness, OutgoingTransaction,
        ParsedBurnData, ValidationOp, ValidationProgram,
    },
};
use nimiq_utils::merkle::MerklePath;
use rand::RngExt;

/// The largest payload a release can carry: `Policy::MAX_TX_SENDER_DATA_SIZE` minus the envelope
/// around it, as pinned by the payload-bounds tests.
const MAX_PAYLOAD_LEN: usize = 4_897;

/// Bytes the little-endian program below reads: 20 address + 8 amount + 8 nonce + 4 + 4.
const LE_LAYOUT_LEN: usize = 44;

/// Bytes the Amoy program reads: 20 address + 32-byte nonce, amount and chain-id words.
const AMOY_LAYOUT_LEN: usize = 116;

/// How long any one sweep may take before it counts as hung. Generous, so a slow CI machine
/// passes; the failure mode being guarded against is infinite.
const HANG_BUDGET: Duration = Duration::from_secs(60);

const RANDOM_PAYLOADS_PER_PROGRAM: usize = 1_000;
const RANDOM_PROGRAMS: usize = 3_000;

/// Little-endian fixed-offset program: target_address [0..20], amount [20..28],
/// target_nonce [28..36], burn_block_height [36..40], target_chain_id [40..44].
fn le_program() -> ValidationProgram {
    ValidationProgram::new(vec![
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
    ])
}

fn config_with(program: ValidationProgram) -> ChainConfig {
    ChainConfig {
        chain_id: 1,
        hash_function: AnyHash::Blake2b(AnyHash32::default()),
        address_format: AddressFormat::Nimiq,
        endianness: Endianness::LittleEndian,
        block_time: Duration::from_secs(60),
        validation_program: program,
        max_proof_depth: 64,
    }
}

fn le_config() -> ChainConfig {
    config_with(le_program())
}

/// A well-formed 44-byte record for `le_program`.
fn le_payload() -> Vec<u8> {
    let mut payload = Vec::with_capacity(LE_LAYOUT_LEN);
    payload.extend_from_slice(&[0xAAu8; 20]);
    payload.extend_from_slice(&500u64.to_le_bytes());
    payload.extend_from_slice(&1u64.to_le_bytes());
    payload.extend_from_slice(&42u32.to_le_bytes());
    payload.extend_from_slice(&1u32.to_le_bytes());
    payload
}

/// A well-formed 116-byte `TokensBurned` record for the Amoy program: one NIM, nonce 1, burnt on
/// Amoy itself.
fn amoy_payload() -> Vec<u8> {
    fn word(value: u128) -> [u8; 32] {
        let mut word = [0u8; 32];
        word[16..].copy_from_slice(&value.to_be_bytes());
        word
    }
    let mut payload = Vec::with_capacity(AMOY_LAYOUT_LEN);
    payload.extend_from_slice(&[0x77u8; 20]);
    payload.extend_from_slice(&word(1));
    payload.extend_from_slice(&word(1_000_000_000_000_000_000));
    payload.extend_from_slice(&word(80_002));
    payload
}

fn release(payload: Vec<u8>) -> OutgoingTransaction {
    OutgoingTransaction {
        burn_transaction_data: payload,
        merkle_proof: AnyMerkleProof::Blake2bPath(MerklePath::empty()),
        oracle_state_index: 0,
    }
}

/// The errors the parser is allowed to fail with. Anything else — and any panic — is a bug.
fn is_typed_rejection(error: &BridgeError) -> bool {
    matches!(
        error,
        BridgeError::InvalidDataLength
            | BridgeError::InvalidRecipientData
            | BridgeError::InvalidAmount
            | BridgeError::InvalidAddress(_)
            | BridgeError::InvalidValidityHeight
            | BridgeError::InvalidChainId
    )
}

/// Invariants every successfully parsed burn satisfies, whatever produced it.
fn assert_well_formed(parsed: &ParsedBurnData, context: &str) {
    assert_ne!(parsed.amount, Coin::ZERO, "{context}: zero amount parsed");
    assert_ne!(parsed.target_nonce, 0, "{context}: zero nonce parsed");
    assert_ne!(
        parsed.burn_block_height, 0,
        "{context}: zero burn height parsed"
    );
    assert_ne!(parsed.target_chain_id, 0, "{context}: zero chain id parsed");
}

/// Runs `body` on a worker thread and fails if it has not finished within `HANG_BUDGET`, so a
/// regression into unbounded work fails the test instead of stalling the whole binary.
fn within_budget<T: Send + 'static>(body: impl FnOnce() -> T + Send + 'static) -> T {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let _ = sender.send(body());
    });
    receiver
        .recv_timeout(HANG_BUDGET)
        .expect("parsing must finish: the program or payload made it hang")
}

// ---------------------------------------------------------------------------------------------
// Truncation: every length short of the layout is rejected, every length at or past it parses
// ---------------------------------------------------------------------------------------------

fn assert_truncation_sweep(config: &ChainConfig, full: &[u8], layout_len: usize, name: &str) {
    let reference = release(full.to_vec())
        .parse_burn_data(config)
        .unwrap_or_else(|e| panic!("{name}: the full payload must parse, got {e:?}"));

    for len in 0..layout_len {
        let short = &full[..len];
        assert!(
            matches!(
                config.validation_program.extract_only(short),
                Err(BridgeError::InvalidDataLength)
            ),
            "{name}: {len}-byte payload must be rejected as too short by the program"
        );
        assert!(
            matches!(
                release(short.to_vec()).parse_burn_data(config),
                Err(BridgeError::InvalidDataLength)
            ),
            "{name}: {len}-byte payload must be rejected as too short on the release path"
        );
    }

    // Trailing bytes are never read: the record parses identically however much filler follows.
    for extra in 0..=64 {
        let mut padded = full.to_vec();
        padded.extend(std::iter::repeat_n(0xEE, extra));
        let parsed = release(padded)
            .parse_burn_data(config)
            .unwrap_or_else(|e| panic!("{name}: {extra} filler bytes must not matter, got {e:?}"));
        assert_eq!(
            parsed.amount, reference.amount,
            "{name}: filler changed the amount"
        );
        assert_eq!(
            parsed.target_address, reference.target_address,
            "{name}: filler changed the address"
        );
        assert_eq!(
            parsed.target_nonce, reference.target_nonce,
            "{name}: filler changed the nonce"
        );
    }
}

#[test]
fn every_truncation_of_a_little_endian_record_is_rejected_as_too_short() {
    assert_truncation_sweep(&le_config(), &le_payload(), LE_LAYOUT_LEN, "le");
}

#[test]
fn every_truncation_of_an_amoy_burn_event_is_rejected_as_too_short() {
    assert_truncation_sweep(
        &amoy_chain_config(),
        &amoy_payload(),
        AMOY_LAYOUT_LEN,
        "amoy",
    );
}

// ---------------------------------------------------------------------------------------------
// The empty payload, and the empty program
// ---------------------------------------------------------------------------------------------

#[test]
fn an_empty_payload_is_rejected_by_every_program_that_reads_it() {
    for (name, config) in [("le", le_config()), ("amoy", amoy_chain_config())] {
        assert!(
            matches!(
                config.validation_program.extract_only(&[]),
                Err(BridgeError::InvalidDataLength)
            ),
            "{name}: the program must reject an empty payload"
        );
        assert!(
            matches!(
                release(Vec::new()).parse_burn_data(&config),
                Err(BridgeError::InvalidDataLength)
            ),
            "{name}: the release path must reject an empty payload"
        );
    }
}

/// A bridge whose program reads nothing at all extracts nothing, so it can never describe a
/// payout — not even for an empty payload, and not for any other one.
#[test]
fn a_program_that_reads_nothing_cannot_describe_a_burn() {
    let config = config_with(ValidationProgram::empty());
    for payload in [Vec::new(), le_payload(), vec![0xFF; MAX_PAYLOAD_LEN]] {
        let result = config.validation_program.extract_only(&payload);
        assert!(
            result.as_ref().is_ok_and(|r| r.extracted_values.is_empty()),
            "an empty program extracts nothing, got {result:?}"
        );
        assert!(
            matches!(
                release(payload).parse_burn_data(&config),
                Err(BridgeError::InvalidRecipientData)
            ),
            "a burn with no fields must be rejected"
        );
    }
}

// ---------------------------------------------------------------------------------------------
// Garbled records of the right length
// ---------------------------------------------------------------------------------------------

/// Full-length Amoy records whose words do not describe a valid burn are rejected with the
/// specific typed error, not a panic: all-ones overflows the amount word, all-zeros is a zero
/// amount.
#[test]
fn garbled_full_length_amoy_records_are_rejected_with_typed_errors() {
    let config = amoy_chain_config();

    let all_ones = release(vec![0xFF; AMOY_LAYOUT_LEN]);
    assert!(
        matches!(
            all_ones.parse_burn_data(&config),
            Err(BridgeError::InvalidAmount)
        ),
        "an amount word that overflows after scaling must be an InvalidAmount"
    );

    let all_zeros = release(vec![0x00; AMOY_LAYOUT_LEN]);
    assert!(
        matches!(
            all_zeros.parse_burn_data(&config),
            Err(BridgeError::InvalidAmount)
        ),
        "a zero amount must be an InvalidAmount"
    );
}

// ---------------------------------------------------------------------------------------------
// Random payloads against the real programs
// ---------------------------------------------------------------------------------------------

fn random_payload(rng: &mut impl RngExt) -> Vec<u8> {
    // Bias towards the interesting region around the layouts while still reaching the ceiling.
    let len = if rng.random_bool(0.5) {
        rng.random_range(0..=128)
    } else {
        rng.random_range(0..=MAX_PAYLOAD_LEN)
    };
    let mut payload = vec![0u8; len];
    rng.fill(&mut payload[..]);
    payload
}

/// Random bytes of random length, from empty to the ceiling, against both real programs: every
/// outcome is either a parsed burn that satisfies the basic invariants or one of the typed
/// rejections. The seed is fixed, so a failure here reproduces.
#[test]
fn random_payloads_against_real_programs_never_panic_and_only_fail_with_typed_errors() {
    within_budget(|| {
        let mut rng = test_rng(true);
        for (name, config) in [("le", le_config()), ("amoy", amoy_chain_config())] {
            for i in 0..RANDOM_PAYLOADS_PER_PROGRAM {
                let payload = random_payload(&mut rng);
                let context = format!("{name} payload #{i} ({} bytes)", payload.len());
                match release(payload).parse_burn_data(&config) {
                    Ok(parsed) => assert_well_formed(&parsed, &context),
                    Err(error) => assert!(
                        is_typed_rejection(&error),
                        "{context}: unexpected error {error:?}"
                    ),
                }
            }
        }
    });
}

// ---------------------------------------------------------------------------------------------
// Random programs against random payloads
// ---------------------------------------------------------------------------------------------

/// Names a random program stores under: the five the release path looks up, plus a scratch name
/// and an empty one.
const NAMES: [&str; 7] = [
    "amount",
    "target_address",
    "target_nonce",
    "burn_block_height",
    "target_chain_id",
    "scratch",
    "",
];

/// Constants that sit on the boundaries the VM cares about: layout offsets, the payload ceiling,
/// and the top of the integer ranges.
const EDGE_CONSTANTS: [u64; 13] = [
    0,
    1,
    19,
    20,
    44,
    52,
    84,
    116,
    MAX_PAYLOAD_LEN as u64,
    u32::MAX as u64,
    u64::MAX - 1,
    u64::MAX,
    10_000_000_000_000,
];

fn random_u64(rng: &mut impl RngExt) -> u64 {
    if rng.random_bool(0.7) {
        EDGE_CONSTANTS[rng.random_range(0..EDGE_CONSTANTS.len())]
    } else {
        rng.random()
    }
}

fn random_name(rng: &mut impl RngExt) -> String {
    NAMES[rng.random_range(0..NAMES.len())].to_string()
}

/// One random operation for position `index` of a program `len` operations long.
fn random_op(rng: &mut impl RngExt, index: usize, len: usize) -> ValidationOp {
    let endianness = if rng.random_bool(0.5) {
        Endianness::LittleEndian
    } else {
        Endianness::BigEndian
    };
    match rng.random_range(0..29) {
        0 => ValidationOp::PushConst(random_u64(rng)),
        1 => ValidationOp::LoadBytes,
        2 => ValidationOp::LoadU64(endianness),
        3 => ValidationOp::LoadU32(endianness),
        4 => ValidationOp::LoadAddress,
        5 => ValidationOp::LoadEvmU64,
        6 => ValidationOp::LoadEvmU64Scaled(random_u64(rng)),
        7 => ValidationOp::PushExpectedAmount,
        8 => ValidationOp::PushExpectedAddress,
        9 => ValidationOp::PushExpectedNonce,
        10 => ValidationOp::PushExpectedValidityHeight,
        11 => ValidationOp::Add,
        12 => ValidationOp::Sub,
        13 => ValidationOp::Mul,
        14 => ValidationOp::Div,
        15 => ValidationOp::Mod,
        16 => ValidationOp::Eq,
        17 => ValidationOp::Lt,
        18 => ValidationOp::Gt,
        19 => ValidationOp::IsZero,
        20 => ValidationOp::And,
        21 => ValidationOp::Or,
        22 => ValidationOp::Not,
        23 => ValidationOp::Dup,
        24 => ValidationOp::Swap,
        25 => ValidationOp::Pop,
        26 => ValidationOp::Assert,
        27 => {
            // Skips that land inside, on the end of, and far beyond the program — including the
            // ones that would wrap the program counter if the jump were not saturating.
            let skips = [
                0,
                1,
                2,
                len - index,
                len,
                usize::MAX - index,
                usize::MAX - 1,
                usize::MAX,
            ];
            ValidationOp::JumpIfZero(skips[rng.random_range(0..skips.len())])
        }
        _ => {
            if rng.random_bool(0.5) {
                ValidationOp::Store(random_name(rng))
            } else {
                ValidationOp::Load(random_name(rng))
            }
        }
    }
}

fn random_program(rng: &mut impl RngExt) -> ValidationProgram {
    let len = rng.random_range(1..=24);
    ValidationProgram::new((0..len).map(|index| random_op(rng, index, len)).collect())
}

/// Programs drawn from the whole opcode set with boundary immediates, run over random payloads
/// through both the bare VM and the release path. Every run terminates, never panics, and fails
/// only with a typed rejection; every success satisfies the burn invariants. Program-counter
/// wraps are pinned deterministically in the VM's own tests; here they simply show up in the mix.
#[test]
fn random_programs_over_random_payloads_terminate_and_only_fail_with_typed_errors() {
    within_budget(|| {
        let mut rng = test_rng(true);
        for i in 0..RANDOM_PROGRAMS {
            let program = random_program(&mut rng);
            let payload = random_payload(&mut rng);
            let context = format!(
                "program #{i} {:?} over {} bytes",
                program.operations,
                payload.len()
            );

            if let Err(error) = program.extract_only(&payload) {
                assert!(
                    is_typed_rejection(&error),
                    "{context}: unexpected VM error {error:?}"
                );
            }

            match release(payload).parse_burn_data(&config_with(program)) {
                Ok(parsed) => assert_well_formed(&parsed, &context),
                Err(error) => assert!(
                    is_typed_rejection(&error),
                    "{context}: unexpected release-path error {error:?}"
                ),
            }
        }
    });
}
