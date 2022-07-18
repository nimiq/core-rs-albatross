use lazy_static::lazy_static;

use beserial::{Deserialize, Serialize};
use nimiq_keys::{Address, KeyPair, PrivateKey};
use nimiq_primitives::coin::Coin;
use nimiq_primitives::networks::NetworkId;
use nimiq_test_log::test;
use nimiq_wallet::WalletAccount;

lazy_static! {
    /// This is an example for using doc comment attributes
    ///
    /// Address:       NQ58 KKC2 JJ35 1N82 T5EM 5SHE R4FA UTFB 1JFX
    /// Address (raw): 9cd82948650d902d95d52ea2ec91eae6deb0c9fe
    /// Public Key:    7f07b8a4c2f6c2f7cb56584a00672af88733cb6f80f5d6e6cf4043a3d4aeec05
    /// Private Key:   b410a7a583cbc13ef4f1cbddace30928bcb4f9c13722414bc4a2faaba3f4e187
    static ref WALLET: WalletAccount = {
        let raw_private_key = hex::decode("b410a7a583cbc13ef4f1cbddace30928bcb4f9c13722414bc4a2faaba3f4e187").unwrap();
        let private_key: PrivateKey = Deserialize::deserialize_from_vec(&raw_private_key).unwrap();
        WalletAccount::from(KeyPair::from(private_key))
    };
}

#[test]
fn test_create_transaction() {
    let wallet = WALLET.clone();
    let transaction = wallet.create_transaction(
        Address::from_user_friendly_address("NQ16 C3HR 85U8 P7MK F52R E9RG SA3Y Q69C X563")
            .unwrap(),
        Coin::from_u64_unchecked(42),
        Coin::ZERO,
        0,
        Some(NetworkId::Main),
    );
    assert_eq!(Ok(()), transaction.verify(NetworkId::Main));
}

#[test]
fn test_serialize_deserialize() {
    let wallet = WALLET.clone();
    let serialized = wallet.serialize_to_vec();
    match WalletAccount::deserialize_from_vec(&serialized) {
        Ok(deserialized) => {
            assert_eq!(wallet, deserialized);
        }
        Err(e) => {
            assert!(false, "Error: {}", e);
        }
    }
}
