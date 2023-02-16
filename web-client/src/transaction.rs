use wasm_bindgen::prelude::*;

use beserial::Serialize;
use nimiq_hash::{Blake2bHash, Hash};
use nimiq_primitives::coin::Coin;

use crate::address::Address;
use crate::utils::{from_network_id, to_network_id};

#[wasm_bindgen]
pub struct Transaction {
    inner: nimiq_transaction::Transaction,
}

#[wasm_bindgen]
impl Transaction {
    pub fn basic(
        sender: &Address,
        recipient: &Address,
        value: u64,
        fee: u64,
        validity_start_height: u32,
        network_id: u8,
    ) -> Result<Transaction, JsError> {
        Ok(Transaction::from_native(
            nimiq_transaction::Transaction::new_basic(
                sender.native_ref().clone(),
                recipient.native_ref().clone(),
                Coin::try_from(value)?,
                Coin::try_from(fee)?,
                validity_start_height,
                to_network_id(network_id)?,
            ),
        ))
    }

    pub fn hash(&self) -> String {
        let hash: Blake2bHash = self.inner.hash();
        // TODO: Return an instance of Hash
        hash.to_hex()
    }

    pub fn verify(&self, network_id: Option<u8>) -> Result<(), JsError> {
        let network_id = match network_id {
            Some(id) => to_network_id(id)?,
            None => self.inner.network_id,
        };

        self.inner.verify(network_id).map_err(JsError::from)
    }

    #[wasm_bindgen(js_name = isValidAt)]
    pub fn is_valid_at(&self, block_height: u32) -> bool {
        self.inner.is_valid_at(block_height)
    }

    #[wasm_bindgen(js_name = getContractCreationAddress)]
    pub fn get_contract_creation_address(&self) -> Address {
        Address::from_native(self.inner.contract_creation_address())
    }

    #[wasm_bindgen(js_name = serializeContent)]
    pub fn serialize_content(&self) -> Vec<u8> {
        self.inner.serialize_content()
    }

    pub fn serialize(&self) -> Vec<u8> {
        self.inner.serialize_to_vec()
    }

    #[wasm_bindgen(getter)]
    pub fn sender(&self) -> Address {
        Address::from_native(self.inner.sender.clone())
    }

    #[wasm_bindgen(getter, js_name = senderType)]
    pub fn sender_type(&self) -> u8 {
        self.inner.sender_type.into()
    }

    #[wasm_bindgen(getter)]
    pub fn recipient(&self) -> Address {
        Address::from_native(self.inner.recipient.clone())
    }

    #[wasm_bindgen(getter, js_name = recipientType)]
    pub fn recipient_type(&self) -> u8 {
        self.inner.recipient_type.into()
    }

    #[wasm_bindgen(getter)]
    pub fn value(&self) -> u64 {
        self.inner.value.into()
    }

    #[wasm_bindgen(getter)]
    pub fn fee(&self) -> u64 {
        self.inner.fee.into()
    }

    #[wasm_bindgen(getter, js_name = validityStartHeight)]
    pub fn validity_start_height(&self) -> u32 {
        self.inner.validity_start_height
    }

    #[wasm_bindgen(getter, js_name = networkId)]
    pub fn network_id(&self) -> u8 {
        from_network_id(self.inner.network_id)
    }

    #[wasm_bindgen(getter)]
    pub fn flags(&self) -> u8 {
        self.inner.flags.into()
    }

    #[wasm_bindgen(getter)]
    pub fn proof(&self) -> Vec<u8> {
        self.inner.proof.clone()
    }

    #[wasm_bindgen(setter)]
    pub fn set_proof(&mut self, proof: Vec<u8>) {
        self.inner.proof = proof;
    }

    #[wasm_bindgen(getter)]
    pub fn serialized_size(&self) -> usize {
        self.inner.serialized_size()
    }
}

impl Transaction {
    pub fn from_native(transaction: nimiq_transaction::Transaction) -> Transaction {
        Transaction { inner: transaction }
    }

    pub fn native_ref(&self) -> &nimiq_transaction::Transaction {
        &self.inner
    }
}
