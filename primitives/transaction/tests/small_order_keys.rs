use nimiq_bls::KeyPair as BlsKeyPair;
use nimiq_keys::{Address, Ed25519PublicKey, KeyPair};
use nimiq_primitives::{account::AccountType, coin::Coin, networks::NetworkId};
use nimiq_serde::Serialize;
use nimiq_test_log::test;
use nimiq_test_utils::test_rng;
use nimiq_transaction::{
    account::{
        htlc_contract::OutgoingHTLCTransactionProof,
        staking_contract::IncomingStakingTransactionData,
    },
    SignatureProof, Transaction,
};
use nimiq_utils::key_rng::SecureGenerate;

/// The identity point: a small-order key the all-zero-key guard doesn't catch.
fn small_order_key() -> Ed25519PublicKey {
    let mut bytes = [0u8; Ed25519PublicKey::SIZE];
    bytes[0] = 1;
    Ed25519PublicKey::from(bytes)
}

/// The wildcard proof: a small-order key with the all-zero signature.
fn small_order_proof() -> SignatureProof {
    SignatureProof::from_ed25519(small_order_key(), Default::default())
}

fn honest_proof(key_pair: &KeyPair) -> SignatureProof {
    SignatureProof::from_ed25519(key_pair.public, key_pair.sign(b"message"))
}

fn transaction(
    sender_type: AccountType,
    recipient_data: Option<IncomingStakingTransactionData>,
) -> Transaction {
    let recipient_type = match recipient_data {
        Some(_) => AccountType::Staking,
        None => AccountType::Basic,
    };
    Transaction::new_extended(
        Address::from([1u8; Address::SIZE]),
        sender_type,
        vec![],
        Address::from([2u8; Address::SIZE]),
        recipient_type,
        recipient_data.map_or_else(Vec::new, |data| data.serialize_to_vec()),
        Coin::from_u64_unchecked(100),
        Coin::ZERO,
        1,
        NetworkId::UnitAlbatross,
    )
}

#[test]
fn it_flags_a_small_order_sender_proof() {
    let key_pair = KeyPair::generate(&mut test_rng(false));
    let mut tx = transaction(AccountType::Basic, None);

    tx.proof = small_order_proof().serialize_to_vec();
    assert!(tx.uses_small_order_key());

    tx.proof = honest_proof(&key_pair).serialize_to_vec();
    assert!(!tx.uses_small_order_key());
}

#[test]
fn it_flags_a_small_order_htlc_proof() {
    let key_pair = KeyPair::generate(&mut test_rng(false));
    let mut tx = transaction(AccountType::HTLC, None);

    // Only one of the two early-resolve proofs is weak.
    tx.proof = OutgoingHTLCTransactionProof::EarlyResolve {
        signature_proof_recipient: honest_proof(&key_pair),
        signature_proof_sender: small_order_proof(),
    }
    .serialize_to_vec();
    assert!(tx.uses_small_order_key());

    tx.proof = OutgoingHTLCTransactionProof::TimeoutResolve {
        signature_proof_sender: honest_proof(&key_pair),
    }
    .serialize_to_vec();
    assert!(!tx.uses_small_order_key());
}

#[test]
fn it_flags_a_small_order_staking_proof() {
    let key_pair = KeyPair::generate(&mut test_rng(false));
    let create_staker = |proof| {
        transaction(
            AccountType::Basic,
            Some(IncomingStakingTransactionData::CreateStaker {
                delegation: None,
                proof,
            }),
        )
    };

    assert!(!create_staker(honest_proof(&key_pair)).uses_small_order_key());
    // A wildcard staker proof would create a staker anyone can control.
    assert!(create_staker(small_order_proof()).uses_small_order_key());
}

#[test]
fn it_flags_a_small_order_validator_signing_key() {
    let mut rng = test_rng(false);
    let key_pair = KeyPair::generate(&mut rng);
    let voting_key_pair = BlsKeyPair::generate(&mut rng);
    let voting_key = voting_key_pair.public_key.compress();

    let create_validator = |signing_key| {
        transaction(
            AccountType::Basic,
            Some(IncomingStakingTransactionData::CreateValidator {
                signing_key,
                voting_key: voting_key.clone(),
                reward_address: Address::from(&key_pair),
                signal_data: None,
                proof_of_knowledge: voting_key_pair
                    .sign(&voting_key.serialize_to_vec())
                    .compress(),
                proof: honest_proof(&key_pair),
            }),
        )
    };
    assert!(!create_validator(key_pair.public).uses_small_order_key());
    assert!(create_validator(small_order_key()).uses_small_order_key());

    let update_validator = |new_signing_key| {
        transaction(
            AccountType::Basic,
            Some(IncomingStakingTransactionData::UpdateValidator {
                new_signing_key,
                new_voting_key: None,
                new_reward_address: None,
                new_signal_data: None,
                new_proof_of_knowledge: None,
                proof: honest_proof(&key_pair),
            }),
        )
    };
    assert!(!update_validator(None).uses_small_order_key());
    assert!(!update_validator(Some(key_pair.public)).uses_small_order_key());
    assert!(update_validator(Some(small_order_key())).uses_small_order_key());
}
