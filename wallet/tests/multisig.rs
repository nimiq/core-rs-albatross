use std::num::NonZeroU8;

use hex::FromHex;
use nimiq_keys::{multisig::Commitment, Address, KeyPair, PrivateKey, PublicKey};
use nimiq_primitives::{coin::Coin, networks::NetworkId};
use nimiq_wallet::MultiSigAccount;

static PRIVATE_KEYS: &'static [&str] = &[
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

    let commitment_pair1 = multi_sig_1.create_commitment();
    let commitment_pair2 = multi_sig_2.create_commitment();

    let commitments = vec![
        *commitment_pair1.commitment(),
        *commitment_pair2.commitment(),
    ];

    let aggregated_commitment: Commitment = commitments.iter().sum();

    let aggregated_public_key: PublicKey =
        MultiSigAccount::aggregate_public_keys(&vec![kp1.public.clone(), kp2.public.clone()]);

    let transaction = multi_sig_1.create_transaction(
        Address::from_any_str(&"NQ68 D40E KU4Q V8JV E96E X1M1 5NL6 KUYC SQXS").unwrap(),
        Coin::from_u64_unchecked(1),
        Coin::ZERO,
        1,
        NetworkId::Dummy,
    );

    let partial_signature1 = multi_sig_1.partially_sign_transaction(
        &transaction,
        &public_keys,
        &commitments,
        &commitment_pair1.random_secret(),
    );

    let partial_signature2 = multi_sig_2.partially_sign_transaction(
        &transaction,
        &public_keys,
        &commitments,
        &commitment_pair2.random_secret(),
    );

    let tx = multi_sig_1
        .sign_transaction(
            &transaction,
            &aggregated_public_key,
            &aggregated_commitment,
            &vec![partial_signature1, partial_signature2],
        )
        .unwrap();

    assert!(tx.verify(NetworkId::Dummy).is_ok())
}

#[test]
pub fn empty_public_keys_list_not_allowed() {
    let kp = KeyPair::from(PrivateKey::from_hex(PRIVATE_KEYS[0]).unwrap());

    let multi_sig = MultiSigAccount::from_public_keys(&kp, NonZeroU8::new(1).unwrap(), &vec![]);
    assert!(multi_sig.is_err());

    let multi_sig =
        MultiSigAccount::from_public_keys(&kp, NonZeroU8::new(1).unwrap(), &vec![kp.public]);
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
        &vec![kp2.public, kp3.public],
    );
    assert!(multi_sig.is_err());

    let multi_sig = MultiSigAccount::from_public_keys(
        &kp1,
        NonZeroU8::new(3).unwrap(),
        &vec![kp1.public, kp2.public, kp3.public],
    );
    assert!(multi_sig.is_ok());
}

#[test]
pub fn aggregated_public_key_order_does_not_matter() {
    let kp1 = KeyPair::from(PrivateKey::from_hex(PRIVATE_KEYS[0]).unwrap());
    let kp2 = KeyPair::from(PrivateKey::from_hex(PRIVATE_KEYS[1]).unwrap());
    let kp3 = KeyPair::from(PrivateKey::from_hex(PRIVATE_KEYS[2]).unwrap());

    let pk1 = MultiSigAccount::aggregate_public_keys(&vec![kp1.public, kp2.public, kp3.public]);
    let pk2 = MultiSigAccount::aggregate_public_keys(&vec![kp2.public, kp3.public, kp1.public]);
    let pk3 = MultiSigAccount::aggregate_public_keys(&vec![kp3.public, kp2.public, kp1.public]);

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
        &vec![kp1.public, kp2.public],
    );
    let multi_sig_2 = MultiSigAccount::from_public_keys(
        &kp1,
        NonZeroU8::new(2).unwrap(),
        &vec![kp1.public, kp2.public],
    );

    assert_eq!(multi_sig_1.unwrap().address, multi_sig_2.unwrap().address);
}

#[test]
pub fn it_correctly_computes_merkle_root() {
    let kp1 = KeyPair::from(PrivateKey::from_hex(PRIVATE_KEYS[0]).unwrap());
    let kp2 = KeyPair::from(PrivateKey::from_hex(PRIVATE_KEYS[1]).unwrap());

    let multi_sig = MultiSigAccount::from_public_keys(
        &kp1,
        NonZeroU8::new(2).unwrap(),
        &vec![kp1.public, kp2.public],
    );

    assert_eq!(
        multi_sig.unwrap().address,
        Address::from_any_str("4de9f6fe2e188b50eaef60f08322d455b65e51ea").unwrap()
    );
}
