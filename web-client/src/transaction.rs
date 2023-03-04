use serde::ser::SerializeStruct;
use wasm_bindgen::prelude::*;

use beserial::Serialize;
use nimiq_hash::{Blake2bHash, Hash};
use nimiq_primitives::{account::AccountType, coin::Coin};
use nimiq_transaction::{
    extended_transaction::ExtendedTransaction, TransactionFlags, TransactionFormat,
};
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

    /// The transaction's format: `0` = Basic, `1` = Extended.
    #[wasm_bindgen(getter)]
    pub fn format(&self) -> u8 {
        match self.inner.format() {
            TransactionFormat::Basic => 0,
            TransactionFormat::Extended => 1,
        }
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

    /// The transaction's fee per byte in luna (NIM's smallest unit).
    #[wasm_bindgen(getter, js_name = feePerByte)]
    pub fn fee_per_byte(&self) -> f64 {
        self.inner.fee_per_byte()
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

    /// The transaction's data as a byte array.
    #[wasm_bindgen(getter)]
    pub fn data(&self) -> Vec<u8> {
        self.inner.data.to_vec()
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

    #[wasm_bindgen(js_name = toPlain)]
    pub fn to_plain(&self) -> Result<PlainTransactionType, JsError> {
        let plain = PlainTransaction::from_transaction(&self.inner);
        Ok(serde_wasm_bindgen::to_value(&plain)?.into())
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

#[derive(serde::Serialize, serde::Deserialize)]
pub struct Raw {
    raw: String,
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlainTransaction {
    pub transaction_hash: String,
    pub format: String,
    pub sender: String,
    pub sender_type: String,
    pub recipient: String,
    pub recipient_type: String,
    pub value: u64,
    pub fee: u64,
    pub fee_per_byte: f64,
    pub validity_start_height: u32,
    pub network: String,
    pub flags: u8,
    pub data: Raw,
    pub proof: Raw,
    pub size: usize,
    pub valid: bool,
}

#[wasm_bindgen(typescript_custom_section)]
const PLAIN_TRANSACTION_TYPE: &'static str = r#"
export enum TransactionFormat {
  BASIC = "basic",
  EXTENDED = "extended",
};

export enum AccountType {
  BASIC = "basic",
  VESTING = "vesting",
  HTLC = "htlc",
  STAKING = "staking",
  VALIDATOR = "validator",
  STAKER = "staker",
};

export type PlainTransaction = {
  transactionHash: string,
  format: TransactionFormat,
  sender: string,
  senderType: AccountType,
  recipient: string,
  recipientType: AccountType,
  value: number,
  fee: number,
  feePerByte: number,
  validityStartHeight: number,
  network: string,
  flags: number,
  data: { raw: string },
  proof: { raw: string },
  size: number,
  valid: boolean,
};
"#;

impl PlainTransaction {
    pub fn from_transaction(tx: &nimiq_transaction::Transaction) -> Self {
        Self {
            transaction_hash: tx.hash::<Blake2bHash>().to_hex(),
            format: match tx.format() {
                TransactionFormat::Basic => "basic".to_string(),
                TransactionFormat::Extended => "extended".to_string(),
            },
            sender: tx.sender.to_user_friendly_address(),
            sender_type: PlainTransaction::account_type_to_string(tx.sender_type),
            recipient: tx.recipient.to_user_friendly_address(),
            recipient_type: PlainTransaction::account_type_to_string(tx.recipient_type),
            value: tx.value.into(),
            fee: tx.fee.into(),
            fee_per_byte: tx.fee_per_byte(),
            validity_start_height: tx.validity_start_height,
            network: tx.network_id.to_string().to_lowercase(),
            flags: tx.flags.into(),
            data: Raw {
                raw: hex::encode(tx.data.clone()),
            },
            proof: Raw {
                raw: hex::encode(tx.proof.clone()),
            },
            size: tx.serialized_size(),
            valid: tx.verify(tx.network_id).is_ok(),
        }
    }

    fn account_type_to_string(ty: AccountType) -> String {
        match ty {
            AccountType::Basic => "basic".to_string(),
            AccountType::Vesting => "vesting".to_string(),
            AccountType::HTLC => "htlc".to_string(),
            AccountType::Staking => "staking".to_string(),
            AccountType::StakingValidator => "validator".to_string(),
            AccountType::StakingValidatorsStaker => "staker".to_string(),
            AccountType::StakingStaker => "staker".to_string(),
        }
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TransactionState {
    New,
    Pending,
    Included,
    Confirmed,
    Invalidated,
    Expired,
}

pub struct PlainTransactionDetails {
    transaction: PlainTransaction,

    pub state: TransactionState,
    pub execution_result: Option<bool>,
    pub block_height: Option<u32>,
    pub confirmations: Option<u32>,
    pub timestamp: Option<u64>,
}

// Manually implement serde::Serialize trait to ensure struct is serialized into a JS Object and not a Map.
//
// Unfortunately, serde cannot serialize a struct that includes a #[serde(flatten)] annotation into an Object,
// and the Github issue for it is closed as "wontfix": https://github.com/serde-rs/serde/issues/1346
impl serde::Serialize for PlainTransactionDetails {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut plain = serializer.serialize_struct("PlainTransactionDetails", 21)?;
        plain.serialize_field("transactionHash", &self.transaction.transaction_hash)?;
        plain.serialize_field("format", &self.transaction.format)?;
        plain.serialize_field("sender", &self.transaction.sender)?;
        plain.serialize_field("senderType", &self.transaction.sender_type)?;
        plain.serialize_field("recipient", &self.transaction.recipient)?;
        plain.serialize_field("recipientType", &self.transaction.recipient_type)?;
        plain.serialize_field("value", &self.transaction.value)?;
        plain.serialize_field("fee", &self.transaction.fee)?;
        plain.serialize_field("feePerByte", &self.transaction.fee_per_byte)?;
        plain.serialize_field(
            "validityStartHeight",
            &self.transaction.validity_start_height,
        )?;
        plain.serialize_field("network", &self.transaction.network)?;
        plain.serialize_field("flags", &self.transaction.flags)?;
        plain.serialize_field("data", &self.transaction.data)?;
        plain.serialize_field("proof", &self.transaction.proof)?;
        plain.serialize_field("size", &self.transaction.size)?;
        plain.serialize_field("valid", &self.transaction.valid)?;

        plain.serialize_field("state", &self.state)?;
        plain.serialize_field("executionResult", &self.execution_result)?;
        plain.serialize_field("blockHeight", &self.block_height)?;
        plain.serialize_field("confirmations", &self.confirmations)?;
        plain.serialize_field("timestamp", &self.timestamp)?;
        plain.end()
    }
}

#[wasm_bindgen(typescript_custom_section)]
const PLAIN_TRANSACTION_DETAILS_TYPE: &'static str = r#"
export enum TransactionState {
  NEW = "new",
  PENDING = "pending",
  INCLUDED = "included",
  CONFIRMED = "confirmed",
  INVALIDATED = "invalidated",
  EXPIRED = "expired",
}

export type PlainTransactionDetails = PlainTransaction & {
  state: TransactionState,
  executionResult?: boolean,
  blockHeight?: number,
  confirmations?: number,
  timestamp?: number,
};
"#;

impl PlainTransactionDetails {
    pub fn from_extended_transaction(
        ext_tx: &ExtendedTransaction,
        current_block: u32,
        last_macro_block: u32,
    ) -> Self {
        let block_number = ext_tx.block_number;
        let block_time = ext_tx.block_time;

        let state = if last_macro_block >= block_number {
            TransactionState::Confirmed
        } else {
            TransactionState::Included
        };

        let executed_transaction = ext_tx.clone().into_transaction().unwrap();

        Self {
            transaction: PlainTransaction::from_transaction(
                executed_transaction.get_raw_transaction(),
            ),
            state,
            execution_result: Some(executed_transaction.succeeded()),
            block_height: Some(block_number),
            timestamp: Some(block_time),
            confirmations: Some(block_number - current_block + 1),
        }
    }
}

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(typescript_type = "PlainTransaction")]
    pub type PlainTransactionType;

    #[wasm_bindgen(typescript_type = "PlainTransactionDetails")]
    pub type PlainTransactionDetailsType;

    #[wasm_bindgen(typescript_type = "PlainTransactionDetails[]")]
    pub type PlainTransactionDetailsArrayType;
}
