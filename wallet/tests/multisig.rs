use std::num::NonZeroU8;

use hex::FromHex;
use nimiq_keys::{
    multisig::{commitment::CommitmentPair, CommitmentsBuilder},
    Address, KeyPair, PrivateKey,
};
use nimiq_primitives::{coin::Coin, networks::NetworkId};
use nimiq_wallet::MultiSigAccount;

static PRIVATE_KEYS: &[&str] = &[
    "37f485f69a33e942b18b79602edb07481880d0b33a7d46adf693633bba7e85e0",
    "fb7789860ab2165b623cb4bda92f99247582320306ed1417bd6283d57d3694ed",
    "122eb25a770f0dc0a1505fd540f518b72b8592b25fcac120f9c6e87ceb1e0274",
];

#[test]
pub fn it_can_create_valid_transactions() {
    let kp1 = KeyPair::from(PrivateKey::from_hex(PRIVATE_KEYS[0]).unwrap());
    let kp2 = KeyPair::from(PrivateKey::from_hex(PRIVATE_KEYS[1]).unwrap());

    let public_keys = vec![kp1.public, kp2.public];

    let multi_sig_1 =
        MultiSigAccount::from_public_keys(&kp1, NonZeroU8::new(2).unwrap(), &public_keys).unwrap();
    let multi_sig_2 =
        MultiSigAccount::from_public_keys(&kp2, NonZeroU8::new(2).unwrap(), &public_keys).unwrap();

    let commitment_pairs1 = multi_sig_1.create_commitments();
    let commitment_pairs2 = multi_sig_2.create_commitments();

    let transaction = multi_sig_1.create_transaction(
        Address::from_any_str("NQ68 D40E KU4Q V8JV E96E X1M1 5NL6 KUYC SQXS").unwrap(),
        Coin::from_u64_unchecked(1),
        Coin::ZERO,
        1,
        NetworkId::Dummy,
    );

    let data1 = CommitmentsBuilder::with_private_commitments(kp1.public, commitment_pairs1)
        .with_signer(
            kp2.public,
            CommitmentPair::to_commitments(&commitment_pairs2),
        )
        .build(&transaction.serialize_content());
    let data2 = CommitmentsBuilder::with_private_commitments(kp2.public, commitment_pairs2)
        .with_signer(
            kp1.public,
            CommitmentPair::to_commitments(&commitment_pairs1),
        )
        .build(&transaction.serialize_content());

    let partial_signature1 = multi_sig_1
        .partially_sign_transaction(&transaction, &data1)
        .unwrap();
    let partial_signature2 = multi_sig_2
        .partially_sign_transaction(&transaction, &data2)
        .unwrap();

    let tx = multi_sig_1
        .sign_transaction(
            &transaction,
            &data1.aggregate_public_key,
            &data1.aggregate_commitment,
            &[partial_signature1, partial_signature2],
        )
        .unwrap();

    assert!(tx.verify(NetworkId::Dummy).is_ok())
}

#[test]
pub fn empty_public_keys_list_not_allowed() {
    let kp = KeyPair::from(PrivateKey::from_hex(PRIVATE_KEYS[0]).unwrap());

    let multi_sig = MultiSigAccount::from_public_keys(&kp, NonZeroU8::new(1).unwrap(), &[]);
    assert!(multi_sig.is_err());

    let multi_sig =
        MultiSigAccount::from_public_keys(&kp, NonZeroU8::new(1).unwrap(), &[kp.public]);
    assert!(multi_sig.is_ok());
}

#[test]
pub fn owner_keypair_must_be_part_of_public_keys() {
    let kp1 = KeyPair::from(PrivateKey::from_hex(PRIVATE_KEYS[0]).unwrap());
    let kp2 = KeyPair::from(PrivateKey::from_hex(PRIVATE_KEYS[1]).unwrap());
    let kp3 = KeyPair::from(PrivateKey::from_hex(PRIVATE_KEYS[2]).unwrap());

    let multi_sig = MultiSigAccount::from_public_keys(
        &kp1,
        NonZeroU8::new(2).unwrap(),
        &[kp2.public, kp3.public],
    );
    assert!(multi_sig.is_err());

    let multi_sig = MultiSigAccount::from_public_keys(
        &kp1,
        NonZeroU8::new(3).unwrap(),
        &[kp1.public, kp2.public, kp3.public],
    );
    assert!(multi_sig.is_ok());
}

#[test]
pub fn aggregated_public_key_order_does_not_matter() {
    let kp1 = KeyPair::from(PrivateKey::from_hex(PRIVATE_KEYS[0]).unwrap());
    let kp2 = KeyPair::from(PrivateKey::from_hex(PRIVATE_KEYS[1]).unwrap());
    let kp3 = KeyPair::from(PrivateKey::from_hex(PRIVATE_KEYS[2]).unwrap());

    let pk1 = MultiSigAccount::aggregate_public_keys(&[kp1.public, kp2.public, kp3.public]);
    let pk2 = MultiSigAccount::aggregate_public_keys(&[kp2.public, kp3.public, kp1.public]);
    let pk3 = MultiSigAccount::aggregate_public_keys(&[kp3.public, kp2.public, kp1.public]);

    assert_eq!(pk1, pk2);
    assert_eq!(pk1, pk3);
}

#[test]
pub fn from_public_keys_order_does_not_matter() {
    let kp1 = KeyPair::from(PrivateKey::from_hex(PRIVATE_KEYS[0]).unwrap());
    let kp2 = KeyPair::from(PrivateKey::from_hex(PRIVATE_KEYS[1]).unwrap());

    let multi_sig_1 = MultiSigAccount::from_public_keys(
        &kp1,
        NonZeroU8::new(2).unwrap(),
        &[kp1.public, kp2.public],
    )
    .unwrap();
    let multi_sig_2 = MultiSigAccount::from_public_keys(
        &kp1,
        NonZeroU8::new(2).unwrap(),
        &[kp2.public, kp1.public],
    )
    .unwrap();

    assert_eq!(multi_sig_1.address, multi_sig_2.address);
}

#[test]
pub fn it_correctly_computes_merkle_root() {
    let kp1 = KeyPair::from(PrivateKey::from_hex(PRIVATE_KEYS[0]).unwrap());
    let kp2 = KeyPair::from(PrivateKey::from_hex(PRIVATE_KEYS[1]).unwrap());

    let multi_sig = MultiSigAccount::from_public_keys(
        &kp1,
        NonZeroU8::new(2).unwrap(),
        &[kp1.public, kp2.public],
    );

    assert_eq!(
        multi_sig.unwrap().address,
        Address::from_any_str("4de9f6fe2e188b50eaef60f08322d455b65e51ea").unwrap()
    );
}
