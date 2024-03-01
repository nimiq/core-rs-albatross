mod address;
#[cfg(feature = "client")]
mod client;
mod client_configuration;
#[cfg(feature = "primitives")]
mod primitives;
mod transaction;
mod utils;

#[cfg(test)]
mod tests {
    use wasm_bindgen::JsValue;
    use wasm_bindgen_test::wasm_bindgen_test;

    use crate::{
        address::Address,
        primitives::{
            bls_key_pair::BLSKeyPair, key_pair::KeyPair, transaction_builder::TransactionBuilder,
        },
    };

    #[wasm_bindgen_test]
    pub fn it_can_create_and_sign_basic_transactions() {
        let keypair = KeyPair::generate();

        let mut tx = TransactionBuilder::new_basic(
            &keypair.to_address(),
            &Address::from_string("0000000000000000000000000000000000000000")
                .map_err(JsValue::from)
                .unwrap(),
            100_00000,
            None,
            1,
            5,
        )
        .map_err(JsValue::from)
        .unwrap();

        tx.sign(&keypair).map_err(JsValue::from).unwrap();
        assert_eq!(tx.verify(None).map_err(JsValue::from), Ok(()))
    }

    #[wasm_bindgen_test]
    pub fn it_can_create_and_sign_validator_transactions() {
        let keypair = KeyPair::generate();

        let mut tx = TransactionBuilder::new_create_validator(
            &keypair.to_address(),
            &keypair.to_address(),
            &keypair.public_key(),
            &BLSKeyPair::generate(),
            None,
            None,
            1,
            5,
        )
        .map_err(JsValue::from)
        .unwrap();

        tx.sign(&keypair).map_err(JsValue::from).unwrap();
        assert_eq!(tx.verify(None).map_err(JsValue::from), Ok(()))
    }
}
