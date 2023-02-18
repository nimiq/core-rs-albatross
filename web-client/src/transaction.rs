use wasm_bindgen::prelude::*;

use beserial::Serialize;
use nimiq_hash::{Blake2bHash, Hash};
use nimiq_primitives::coin::Coin;

use crate::address::Address;
use crate::utils::{from_network_id, to_network_id};

/// Transactions describe a transfer of value, usually from the sender to the recipient.
/// However, transactions can also have no value, when they are used to _signal_ a change in the staking contract.
///
/// Transactions can be used to create contracts, such as vesting contracts and HTLCs.
///
/// Transactions require a valid signature proof over their serialized content.
/// Furthermore, transactions are only valid for 2 hours after their validity-start block height.
#[wasm_bindgen]
pub struct Transaction {
    inner: nimiq_transaction::Transaction,
}

#[wasm_bindgen]
impl Transaction {
    /// Creates a basic transaction that simply transfers `value` amount of luna (NIM's smallest unit)
    /// from the sender to the recipient.
    ///
    /// The returned transaction is not yet signed. You can sign it e.g. with `KeyPair.signTransaction`.
    ///
    /// Throws when the numbers given for value and fee do not fit within a u64 or the networkId is unknown.
    #[wasm_bindgen(js_name = newBasicTransaction)]
    pub fn new_basic_transaction(
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

    /// Computes the transaction's hash, which is used as its unique identifier on the blockchain.
    pub fn hash(&self) -> String {
        let hash: Blake2bHash = self.inner.hash();
        // TODO: Return an instance of Hash
        hash.to_hex()
    }

    /// Verifies that a transaction has valid properties and a valid signature proof.
    /// Optionally checks if the transaction is valid on the provided network.
    ///
    /// **Throws with any transaction validity error.** Returns without exception if the transaction is valid.
    ///
    /// Throws when the given networkId is unknown.
    pub fn verify(&self, network_id: Option<u8>) -> Result<(), JsError> {
        let network_id = match network_id {
            Some(id) => to_network_id(id)?,
            None => self.inner.network_id,
        };

        self.inner.verify(network_id).map_err(JsError::from)
    }

    /// Tests if the transaction is valid at the specified block height.
    #[wasm_bindgen(js_name = isValidAt)]
    pub fn is_valid_at(&self, block_height: u32) -> bool {
        self.inner.is_valid_at(block_height)
    }

    /// Returns the address of the contract that is created with this transaction.
    #[wasm_bindgen(js_name = getContractCreationAddress)]
    pub fn get_contract_creation_address(&self) -> Address {
        Address::from_native(self.inner.contract_creation_address())
    }

    /// Serializes the transaction's content to be used for creating its signature.
    #[wasm_bindgen(js_name = serializeContent)]
    pub fn serialize_content(&self) -> Vec<u8> {
        self.inner.serialize_content()
    }

    /// Serializes the transaction to a byte array.
    pub fn serialize(&self) -> Vec<u8> {
        self.inner.serialize_to_vec()
    }

    /// The transaction's sender address.
    #[wasm_bindgen(getter)]
    pub fn sender(&self) -> Address {
        Address::from_native(self.inner.sender.clone())
    }

    /// The transaction's sender type: `0` = Basic, `1` = Vesting, `2` = HTLC, `3` = Staking contract.
    #[wasm_bindgen(getter, js_name = senderType)]
    pub fn sender_type(&self) -> u8 {
        self.inner.sender_type.into()
    }

    /// The transaction's recipient address.
    #[wasm_bindgen(getter)]
    pub fn recipient(&self) -> Address {
        Address::from_native(self.inner.recipient.clone())
    }

    /// The transaction's recipient type: `0` = Basic, `1` = Vesting, `2` = HTLC, `3` = Staking contract.
    #[wasm_bindgen(getter, js_name = recipientType)]
    pub fn recipient_type(&self) -> u8 {
        self.inner.recipient_type.into()
    }

    /// The transaction's value in luna (NIM's smallest unit).
    #[wasm_bindgen(getter)]
    pub fn value(&self) -> u64 {
        self.inner.value.into()
    }

    /// The transaction's fee in luna (NIM's smallest unit).
    #[wasm_bindgen(getter)]
    pub fn fee(&self) -> u64 {
        self.inner.fee.into()
    }

    /// The transaction's validity-start height. The transaction is valid for 2 hours after this block height.
    #[wasm_bindgen(getter, js_name = validityStartHeight)]
    pub fn validity_start_height(&self) -> u32 {
        self.inner.validity_start_height
    }

    /// The transaction's network ID.
    #[wasm_bindgen(getter, js_name = networkId)]
    pub fn network_id(&self) -> u8 {
        from_network_id(self.inner.network_id)
    }

    /// The transaction's flags: `0b1` = contract creation, `0b10` = signalling.
    #[wasm_bindgen(getter)]
    pub fn flags(&self) -> u8 {
        self.inner.flags.into()
    }

    /// The transaction's signature proof as a byte array.
    #[wasm_bindgen(getter)]
    pub fn proof(&self) -> Vec<u8> {
        self.inner.proof.clone()
    }

    /// Set the transaction's signature proof.
    #[wasm_bindgen(setter)]
    pub fn set_proof(&mut self, proof: Vec<u8>) {
        self.inner.proof = proof;
    }

    /// The transaction's byte size.
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
