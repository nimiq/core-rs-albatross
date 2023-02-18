use wasm_bindgen::prelude::*;

use beserial::Serialize;
use nimiq_hash::{Blake2bHash, Hash};
use nimiq_primitives::{account::AccountType, coin::Coin};
use nimiq_transaction::TransactionFlags;
use nimiq_transaction_builder::TransactionProofBuilder;

use crate::address::Address;
use crate::key_pair::KeyPair;
use crate::utils::{from_network_id, to_network_id};

/// Transactions describe a transfer of value, usually from the sender to the recipient.
/// However, transactions can also have no value, when they are used to _signal_ a change in the staking contract.
///
/// Transactions can be used to create contracts, such as vesting contracts and HTLCs.
///
/// Transactions require a valid signature proof over their serialized content.
/// Furthermore, transactions are only valid for 2 hours after their validity-start block height.
#[wasm_bindgen(inspectable)]
pub struct Transaction {
    inner: nimiq_transaction::Transaction,
}

#[wasm_bindgen]
impl Transaction {
    /// Creates a new unsigned transaction that transfers `value` amount of luna (NIM's smallest unit)
    /// from the sender to the recipient, where both sender and recipient can be any account type,
    /// and custom extra data can be added to the transaction.
    ///
    /// ### Basic transactions
    /// If both the sender and recipient types are omitted or `0` and both data and flags are empty,
    /// a smaller basic transaction is created.
    ///
    /// ### Extended transactions
    /// If no flags are given, but sender type is not basic (`0`) or data is set, an extended
    /// transaction is created.
    ///
    /// ### Contract creation transactions
    /// To create a new vesting or HTLC contract, set `flags` to `0b1` and specify the contract
    /// type as the `recipient_type`: `1` for vesting, `2` for HTLC. The `data` bytes must have
    /// the correct format of contract creation data for the respective contract type.
    ///
    /// ### Signalling transactions
    /// To interact with the staking contract, signalling transaction are often used to not
    /// transfer any value, but to simply _signal_ a state change instead, such as changing one's
    /// delegation from one validator to another. To create such a transaction, set `flags` to `
    /// 0b10` and populate the `data` bytes accordingly.
    ///
    /// The returned transaction is not yet signed. You can sign it e.g. with `tx.sign(keyPair)`.
    ///
    /// Throws when an account type is unknown, the numbers given for value and fee do not fit
    /// within a u64 or the networkId is unknown. Also throws when no data or recipient type is
    /// given for contract creation transactions, or no data is given for signalling transactions.
    #[wasm_bindgen(constructor)]
    pub fn new(
        sender: &Address,
        sender_type: Option<u8>,
        recipient: &Address,
        recipient_type: Option<u8>,
        value: u64,
        fee: Option<u64>,
        data: Option<Vec<u8>>,
        flags: Option<u8>,
        validity_start_height: u32,
        network_id: u8,
    ) -> Result<Transaction, JsError> {
        let flags = TransactionFlags::try_from(flags.unwrap_or(0b0))?;

        let tx = if flags.is_empty() {
            // This also creates basic transactions
            nimiq_transaction::Transaction::new_extended(
                sender.native_ref().clone(),
                AccountType::try_from(sender_type.unwrap_or(0))?,
                recipient.native_ref().clone(),
                AccountType::try_from(recipient_type.unwrap_or(0))?,
                Coin::try_from(value)?,
                Coin::try_from(fee.unwrap_or(0))?,
                data.unwrap_or(Vec::new()),
                validity_start_height,
                to_network_id(network_id)?,
            )
        } else if flags.contains(TransactionFlags::CONTRACT_CREATION) {
            nimiq_transaction::Transaction::new_contract_creation(
                data.unwrap_throw(),
                sender.native_ref().clone(),
                AccountType::try_from(sender_type.unwrap_or(0))?,
                AccountType::try_from(recipient_type.unwrap_throw())?,
                Coin::try_from(value)?,
                Coin::try_from(fee.unwrap_or(0))?,
                validity_start_height,
                to_network_id(network_id)?,
            )
        } else if flags.contains(TransactionFlags::SIGNALLING) {
            nimiq_transaction::Transaction::new_signalling(
                sender.native_ref().clone(),
                AccountType::try_from(sender_type.unwrap_or(0))?,
                recipient.native_ref().clone(),
                AccountType::try_from(recipient_type.unwrap_or(3))?,
                Coin::try_from(value)?,
                Coin::try_from(fee.unwrap_or(0))?,
                data.unwrap_throw(),
                validity_start_height,
                to_network_id(network_id)?,
            )
        } else {
            return Err(JsError::new("Invalid flags"));
        };

        Ok(Transaction::from_native(tx))
    }

    /// Signs the transaction with the provided key pair. Automatically determines the format
    /// of the signature proof required for the transaction.
    ///
    /// ### Limitations
    /// - HTLC redemption is not supported and will throw.
    /// - Validator deletion transactions are not and cannot be supported.
    /// - For transaction to the staking contract, both signatures are made with the same keypair,
    ///   so it is not possible to interact with a staker that is different from the sender address
    ///   or using a different cold or signing key for validator transactions.
    pub fn sign(&self, key_pair: &KeyPair) -> Result<Transaction, JsError> {
        let proof_builder = TransactionProofBuilder::new(self.native_ref().clone());
        let signed_transaction = match proof_builder {
            TransactionProofBuilder::Basic(mut builder) => {
                builder.sign_with_key_pair(key_pair.native_ref());
                builder.generate().unwrap()
            }
            TransactionProofBuilder::Vesting(mut builder) => {
                builder.sign_with_key_pair(key_pair.native_ref());
                builder.generate().unwrap()
            }
            TransactionProofBuilder::Htlc(mut _builder) => {
                // TODO: Create a separate HTLC signing method that takes the type of proof as an argument
                return Err(JsError::new(
                    "HTLC redemption transactions are not supported",
                ));

                // // Redeem
                // let sig = builder.signature_with_key_pair(key_pair);
                // builder.regular_transfer(hash_algorithm, pre_image, hash_count, hash_root, sig);

                // // Refund
                // let sig = builder.signature_with_key_pair(key_pair);
                // builder.timeout_resolve(sig);

                // // Early resolve
                // builder.early_resolve(htlc_sender_signature, htlc_recipient_signature);

                // // Sign early
                // let sig = builder.signature_with_key_pair(key_pair);
                // return Ok(sig);

                // builder.generate().unwrap()
            }
            TransactionProofBuilder::OutStaking(mut builder) => {
                // There is no way to distinguish between an unstaking and validator-deletion transaction
                // from the transaction itself.
                // Validator-deletion transactions are thus not supported.
                // builder.delete_validator(key_pair.native_ref());

                builder.unstake(key_pair.native_ref());
                builder.generate().unwrap()
            }
            TransactionProofBuilder::InStaking(mut builder) => {
                // It is possible to add an additional argument `secondary_key_pair: Option<&KeyPair>` with
                // https://docs.rs/wasm-bindgen-derive/latest/wasm_bindgen_derive/#optional-arguments.
                // TODO: Support signing for differing staker and validator signing & cold keys.
                let secondary_key_pair: Option<&KeyPair> = None;

                builder.sign_with_key_pair(secondary_key_pair.unwrap_or(key_pair).native_ref());
                let mut builder = builder.generate().unwrap().unwrap_basic();
                builder.sign_with_key_pair(key_pair.native_ref());
                builder.generate().unwrap()
            }
        };

        Ok(Transaction::from_native(signed_transaction))
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
