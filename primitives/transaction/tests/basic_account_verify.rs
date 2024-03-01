use nimiq_keys::{Address, ES256PublicKey, ES256Signature, PublicKey, Signature};
use nimiq_primitives::{account::AccountType, networks::NetworkId, transaction::TransactionError};
use nimiq_transaction::{account::AccountTransactionVerification, SignatureProof, Transaction};

#[test]
fn it_does_not_allow_creation() {
    let owner = Address::from([0u8; 20]);

    let transaction = Transaction::new_contract_creation(
        owner,
        AccountType::Basic,
        vec![],
        AccountType::Basic,
        vec![],
        100.try_into().unwrap(),
        0.try_into().unwrap(),
        0,
        NetworkId::Dummy,
    );

    assert_eq!(
        AccountType::verify_incoming_transaction(&transaction),
        Err(TransactionError::InvalidForRecipient)
    );
}

#[test]
fn it_does_not_allow_signalling() {
    let owner = Address::from([0u8; 20]);

    let transaction = Transaction::new_signaling(
        owner,
        AccountType::Basic,
        Address::from([1u8; 20]),
        AccountType::Basic,
        0.try_into().unwrap(),
        vec![],
        0,
        NetworkId::Dummy,
    );

    assert_eq!(
        AccountType::verify_incoming_transaction(&transaction),
        Err(TransactionError::ZeroValue)
    );
}

#[test]
fn it_can_verify_webauthn_signature_proofs() {
    let signature_proof = SignatureProof::try_from_webauthn(
        PublicKey::ES256(
            ES256PublicKey::from_bytes(
                &hex::decode("02915782665472928bfe72c2869bbbd6bc0c239379d5a150ea5e2b19b205d53659").unwrap(),
            )
            .unwrap(),
        ),
        None,
        Signature::ES256(
            ES256Signature::from_bytes(
                &hex::decode("07b917e958f6fafcad747ac95e20ddf1ac63fc5d99bf4516e902e94591641084015ef7ed46034af18512743a0dcbc7a786aae27110b8cbd1cce81b062bd80c6e").unwrap(),
            )
            .unwrap(),
        ),
            &hex::decode("49960de5880e8c687434170f6476605b8fe4aeb9a28632c7995cf3ba831d97630165019a6c").unwrap(),
            br#"{"type":"webauthn.get","challenge":"4rk3LpNhR-jlyPRHP-xgniidFviD-pbL1hSyh5Nole8","origin":"http://localhost:3000","crossOrigin":false}"#,
    ).unwrap();

    let tx_content = hex::decode("00009a606a88b08f0be5d0d06b34aa58e851ad6aaf0a000000000000000000000000000000000000000000000000000000989680000000000000000000000000050000").unwrap();
    assert!(signature_proof.verify(&tx_content));
}

#[test]
fn it_can_verify_android_chrome_webauthn_signature_proofs() {
    let signature_proof = SignatureProof::try_from_webauthn(
        PublicKey::ES256(
            ES256PublicKey::from_bytes(
                &hex::decode("0327e1f7995bde5df8a22bd9c27833b532d79c2350e61fc9a85621d1438eabeb7c").unwrap(),
            )
            .unwrap(),
        ),
        None,
        Signature::ES256(
            ES256Signature::from_bytes(
                &hex::decode("a4fe6e4e2990335d2e4ceeaf63ee149e2dc2e0703bc26f6323f4bebb454c7b505f5faf4fc5a47ea89bedf9d37786ce7e5355b179bdf13e9771ce426f13867a9d").unwrap(),
            )
            .unwrap(),
        ),
        &hex::decode("7a03a16dfe0c4358b79eebe4f25cba56ec7aa7c8331f46a96988006db440e6900500000010").unwrap(),
        br#"{"type":"webauthn.get","challenge":"jOG3lhPd8ENEsalR2DSrsLkFO--JT87NHl__MWTVC8c","origin":"https:\/\/webauthn.pos.nimiqwatch.com","androidPackageName":"com.android.chrome"}"#,
    ).unwrap();

    let tx_content = hex::decode("00005f24d6eea3f0299d50dccecfb7a34f8bd5d5168000890c3fee58a9c27ae0f4b5fb9e4a72ee12ccfecf00000000000098968000000000000000000000a7d8060000").unwrap();
    assert!(signature_proof.verify(&tx_content));
}
