extern crate alloc; // Required for wasm-bindgen-derive

#[cfg(feature = "client")]
mod client;
#[cfg(any(feature = "client", feature = "primitives"))]
pub mod common;
#[cfg(any(feature = "crypto", feature = "primitives"))]
mod crypto_utils;
#[cfg(feature = "primitives")]
mod multisig;
#[cfg(feature = "primitives")]
mod primitives;

#[cfg(test)]
mod tests {
    use nimiq_primitives::policy::Policy;
    use wasm_bindgen::JsValue;
    use wasm_bindgen_test::wasm_bindgen_test;

    use crate::{
        common::address::Address,
        primitives::{
            bls_key_pair::BLSKeyPair, key_pair::KeyPair, transaction_builder::TransactionBuilder,
        },
    };

    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

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

        tx.do_sign(&keypair, None).map_err(JsValue::from).unwrap();
        assert_eq!(tx.verify(None, 0).map_err(JsValue::from), Ok(()));
        assert_eq!(
            tx.verify(None, Policy::max_supported_version())
                .map_err(JsValue::from),
            Ok(())
        )
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

        tx.do_sign(&keypair, None).map_err(JsValue::from).unwrap();
        assert_eq!(tx.verify(None, 0).map_err(JsValue::from), Ok(()));
        assert_eq!(
            tx.verify(None, Policy::max_supported_version())
                .map_err(JsValue::from),
            Ok(())
        )
    }
}
