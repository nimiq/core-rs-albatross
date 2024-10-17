use std::str::FromStr;

use nimiq_hash::{Blake2bHash, Hash};
use nimiq_keys::{PublicKey, Signature};
#[cfg(feature = "client")]
use nimiq_primitives::policy::Policy;
use nimiq_primitives::{account::AccountType, coin::Coin, networks::NetworkId};
use nimiq_serde::{Deserialize, Serialize};
use nimiq_transaction::{
    account::{
        htlc_contract::{
            AnyHash, CreationTransactionData as HtlcCreationTransactionData,
            OutgoingHTLCTransactionProof,
        },
        staking_contract::{IncomingStakingTransactionData, OutgoingStakingTransactionData},
        vesting_contract::CreationTransactionData as VestingCreationTransactionData,
    },
    SignatureProof, TransactionFlags, TransactionFormat,
};
#[cfg(feature = "client")]
use nimiq_transaction::{
    historic_transaction::{HistoricTransaction, HistoricTransactionData, RewardEvent},
    ExecutedTransaction,
};
#[cfg(feature = "primitives")]
use nimiq_transaction_builder::TransactionProofBuilder;
#[cfg(feature = "client")]
use serde::ser::SerializeStruct;
use tsify::Tsify;
use wasm_bindgen::prelude::*;
#[cfg(feature = "primitives")]
use wasm_bindgen_derive::TryFromJsValue;

use crate::common::{
    address::Address,
    utils::{from_network_id, to_network_id},
};
#[cfg(feature = "primitives")]
use crate::primitives::key_pair::KeyPair;

/// Transactions describe a transfer of value, usually from the sender to the recipient.
/// However, transactions can also have no value, when they are used to _signal_ a change in the staking contract.
///
/// Transactions can be used to create contracts, such as vesting contracts and HTLCs.
///
/// Transactions require a valid signature proof over their serialized content.
/// Furthermore, transactions are only valid for 2 hours after their validity-start block height.
#[cfg_attr(feature = "primitives", derive(TryFromJsValue))]
#[wasm_bindgen]
#[cfg_attr(feature = "primitives", derive(Clone))]
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
    /// ### Signaling transactions
    /// To interact with the staking contract, signaling transaction are often used to not
    /// transfer any value, but to simply _signal_ a state change instead, such as changing one's
    /// delegation from one validator to another. To create such a transaction, set `flags` to `
    /// 0b10` and populate the `data` bytes accordingly.
    ///
    /// The returned transaction is not yet signed. You can sign it e.g. with `tx.sign(keyPair)`.
    ///
    /// Throws when an account type is unknown, the numbers given for value and fee do not fit
    /// within a u64 or the networkId is unknown. Also throws when no data or recipient type is
    /// given for contract creation transactions, or no data is given for signaling transactions.
    #[wasm_bindgen(constructor)]
    pub fn new(
        sender: &Address,
        sender_type: Option<u8>,
        sender_data: Option<Vec<u8>>,
        recipient: &Address,
        recipient_type: Option<u8>,
        recipient_data: Option<Vec<u8>>,
        value: u64,
        fee: u64,
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
                sender_data.unwrap_or_default(),
                recipient.native_ref().clone(),
                AccountType::try_from(recipient_type.unwrap_or(0))?,
                recipient_data.unwrap_or_default(),
                Coin::try_from(value)?,
                Coin::try_from(fee)?,
                validity_start_height,
                to_network_id(network_id)?,
            )
        } else if flags.contains(TransactionFlags::CONTRACT_CREATION) {
            nimiq_transaction::Transaction::new_contract_creation(
                sender.native_ref().clone(),
                AccountType::try_from(sender_type.unwrap_or(0))?,
                vec![],
                AccountType::try_from(recipient_type.unwrap_throw())?,
                recipient_data.unwrap_throw(),
                Coin::try_from(value)?,
                Coin::try_from(fee)?,
                validity_start_height,
                to_network_id(network_id)?,
            )
        } else if flags.contains(TransactionFlags::SIGNALING) {
            nimiq_transaction::Transaction::new_signaling(
                sender.native_ref().clone(),
                AccountType::try_from(sender_type.unwrap_or(0))?,
                recipient.native_ref().clone(),
                AccountType::try_from(recipient_type.unwrap_or(3))?,
                Coin::try_from(fee)?,
                recipient_data.unwrap_throw(),
                validity_start_height,
                to_network_id(network_id)?,
            )
        } else {
            return Err(JsError::new("Invalid flags"));
        };

        Ok(Transaction::from(tx))
    }

    /// Signs the transaction with the provided key pair. Automatically determines the format
    /// of the signature proof required for the transaction.
    ///
    /// ### Limitations
    /// - HTLC redemption is not supported and will throw.
    /// - For transaction to the staking contract, both signatures are made with the same keypair,
    ///   so it is not possible to interact with a staker that is different from the sender address
    ///   or using a different cold or signing key for validator transactions.
    #[cfg(feature = "primitives")]
    pub fn sign(&mut self, key_pair: &KeyPair) -> Result<(), JsError> {
        let proof_builder = TransactionProofBuilder::new(self.native_ref().clone());
        let proof = match proof_builder {
            TransactionProofBuilder::Basic(mut builder) => {
                builder.sign_with_key_pair(key_pair.native_ref());
                builder.generate().unwrap().proof
            }
            TransactionProofBuilder::Vesting(mut builder) => {
                builder.sign_with_key_pair(key_pair.native_ref());
                builder.generate().unwrap().proof
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
                builder.sign_with_key_pair(key_pair.native_ref());
                builder.generate().unwrap().proof
            }
            TransactionProofBuilder::InStaking(mut builder) => {
                // It is possible to add an additional argument `secondary_key_pair: Option<&KeyPair>` with
                // https://docs.rs/wasm-bindgen-derive/latest/wasm_bindgen_derive/#optional-arguments.
                // TODO: Support signing for differing staker and validator signing & cold keys.
                // let secondary_key_pair: Option<&KeyPair> = None;
                // builder.sign_with_key_pair(secondary_key_pair.unwrap_or(key_pair).native_ref());

                builder.sign_with_key_pair(key_pair.native_ref());
                let mut builder = builder.generate().unwrap().unwrap_basic();
                builder.sign_with_key_pair(key_pair.native_ref());
                let tx = builder.generate().unwrap();
                // Set the recipient data to the data with the added signature
                self.set_recipient_data(tx.recipient_data);
                tx.proof
            }
        };

        self.set_proof(proof);

        Ok(())
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
        Address::from(self.inner.contract_creation_address())
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

    /// The transaction's {@link TransactionFormat}.
    #[wasm_bindgen(getter)]
    pub fn format(&self) -> TransactionFormat {
        self.inner.format()
    }

    /// The transaction's sender address.
    #[wasm_bindgen(getter)]
    pub fn sender(&self) -> Address {
        Address::from(self.inner.sender.clone())
    }

    /// The transaction's sender {@link AccountType}.
    #[wasm_bindgen(getter, js_name = senderType)]
    pub fn sender_type(&self) -> AccountType {
        self.inner.sender_type
    }

    /// The transaction's recipient address.
    #[wasm_bindgen(getter)]
    pub fn recipient(&self) -> Address {
        Address::from(self.inner.recipient.clone())
    }

    /// The transaction's recipient {@link AccountType}.
    #[wasm_bindgen(getter, js_name = recipientType)]
    pub fn recipient_type(&self) -> AccountType {
        self.inner.recipient_type
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

    /// The transaction's flags: `0b1` = contract creation, `0b10` = signaling.
    #[wasm_bindgen(getter)]
    pub fn flags(&self) -> u8 {
        self.inner.flags.into()
    }

    /// The transaction's data as a byte array.
    #[wasm_bindgen(getter, js_name = data)]
    pub fn recipient_data(&self) -> Vec<u8> {
        self.inner.recipient_data.clone()
    }

    /// Set the transaction's data
    #[wasm_bindgen(setter, js_name = data)]
    pub fn set_recipient_data(&mut self, data: Vec<u8>) {
        self.inner.recipient_data = data;
    }

    /// The transaction's sender data as a byte array.
    #[wasm_bindgen(getter, js_name = senderData)]
    pub fn sender_data(&self) -> Vec<u8> {
        self.inner.sender_data.clone()
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
    #[wasm_bindgen(getter, js_name = serializedSize)]
    pub fn serialized_size(&self) -> usize {
        self.inner.serialized_size()
    }

    /// Serializes the transaction into a HEX string.
    #[wasm_bindgen(js_name = toHex)]
    pub fn to_hex(&self) -> String {
        hex::encode(self.serialize())
    }

    /// Creates a JSON-compatible plain object representing the transaction.
    #[wasm_bindgen(js_name = toPlain)]
    pub fn to_plain(&self) -> Result<PlainTransactionType, JsError> {
        let plain = self.to_plain_transaction();
        Ok(serde_wasm_bindgen::to_value(&plain)?.into())
    }

    /// Parses a transaction from a {@link Transaction} instance, a plain object, a hex string
    /// representation, or a byte array.
    ///
    /// Throws when a transaction cannot be parsed from the argument.
    #[wasm_bindgen(js_name = fromAny)]
    pub fn from_any(tx: &TransactionAnyType) -> Result<Transaction, JsError> {
        let js_value: &JsValue = tx.unchecked_ref();

        #[cfg(feature = "primitives")]
        if let Ok(transaction) = Transaction::try_from(js_value) {
            return Ok(transaction);
        }

        if let Ok(plain) = serde_wasm_bindgen::from_value::<PlainTransaction>(js_value.clone()) {
            Ok(Transaction::from_plain_transaction(&plain)?)
        } else if let Ok(string) = serde_wasm_bindgen::from_value::<String>(js_value.to_owned()) {
            Ok(Transaction::from(
                nimiq_transaction::Transaction::deserialize_from_vec(&hex::decode(string)?)?,
            ))
        } else if let Ok(bytes) = serde_wasm_bindgen::from_value::<Vec<u8>>(js_value.to_owned()) {
            Ok(Transaction::from(
                nimiq_transaction::Transaction::deserialize_from_vec(&bytes)?,
            ))
        } else {
            Err(JsError::new("Failed to parse transaction."))
        }
    }

    /// Parses a transaction from a plain object.
    ///
    /// Throws when a transaction cannot be parsed from the argument.
    #[wasm_bindgen(js_name = fromPlain)]
    pub fn from_plain(plain: &PlainTransactionType) -> Result<Transaction, JsError> {
        let js_value: &JsValue = plain.unchecked_ref();

        Transaction::from_plain_transaction(&serde_wasm_bindgen::from_value::<PlainTransaction>(
            js_value.to_owned(),
        )?)
    }
}

impl From<nimiq_transaction::Transaction> for Transaction {
    fn from(transaction: nimiq_transaction::Transaction) -> Self {
        Transaction { inner: transaction }
    }
}

impl Transaction {
    pub fn native_ref(&self) -> &nimiq_transaction::Transaction {
        &self.inner
    }

    #[cfg(feature = "client")]
    pub fn native(&self) -> nimiq_transaction::Transaction {
        self.inner.clone()
    }

    #[cfg(feature = "client")]
    pub fn take_native(self) -> nimiq_transaction::Transaction {
        self.inner
    }

    pub fn to_plain_transaction(&self) -> PlainTransaction {
        PlainTransaction {
            transaction_hash: self.hash(),
            format: self.format(),
            sender: self.sender().to_plain(),
            sender_type: self.sender_type(),
            recipient: self.recipient().to_plain(),
            recipient_type: self.recipient_type(),
            value: self.value(),
            fee: self.fee(),
            fee_per_byte: self.fee_per_byte(),
            validity_start_height: self.validity_start_height(),
            network: to_network_id(self.network_id())
                .ok()
                .unwrap()
                .to_string()
                .to_lowercase(),
            flags: self.flags(),
            sender_data: {
                let raw_data = PlainRawData {
                    raw: hex::encode(self.sender_data()),
                };

                if self.inner.sender_type == AccountType::Staking {
                    let data = OutgoingStakingTransactionData::parse(&self.inner).unwrap();
                    match data {
                        OutgoingStakingTransactionData::DeleteValidator => {
                            PlainTransactionSenderData::DeleteValidator(raw_data)
                        }
                        OutgoingStakingTransactionData::RemoveStake => {
                            PlainTransactionSenderData::RemoveStake(raw_data)
                        }
                    }
                } else {
                    PlainTransactionSenderData::Raw(PlainRawData { raw: raw_data.raw })
                }
            },
            data: {
                if self.inner.recipient_type == AccountType::Staking {
                    // Parse transaction data
                    let data = IncomingStakingTransactionData::parse(&self.inner).unwrap();
                    match data {
                        IncomingStakingTransactionData::CreateStaker {
                            delegation,
                            proof: _proof,
                        } => PlainTransactionRecipientData::CreateStaker(PlainCreateStakerData {
                            raw: hex::encode(self.recipient_data()),
                            delegation: delegation
                                .map(|address| address.to_user_friendly_address()),
                        }),
                        IncomingStakingTransactionData::AddStake { staker_address } => {
                            PlainTransactionRecipientData::AddStake(PlainAddStakeData {
                                raw: hex::encode(self.recipient_data()),
                                staker: staker_address.to_user_friendly_address(),
                            })
                        }
                        IncomingStakingTransactionData::UpdateStaker {
                            new_delegation,
                            reactivate_all_stake,
                            proof: _proof,
                        } => PlainTransactionRecipientData::UpdateStaker(PlainUpdateStakerData {
                            raw: hex::encode(self.recipient_data()),
                            new_delegation: new_delegation
                                .map(|address| address.to_user_friendly_address()),
                            reactivate_all_stake,
                        }),
                        IncomingStakingTransactionData::CreateValidator {
                            signing_key,
                            voting_key,
                            reward_address,
                            signal_data,
                            proof_of_knowledge,
                            proof: _proof,
                        } => PlainTransactionRecipientData::CreateValidator(
                            PlainCreateValidatorData {
                                raw: hex::encode(self.recipient_data()),
                                signing_key: signing_key.to_hex(),
                                voting_key: voting_key.to_hex(),
                                reward_address: reward_address.to_user_friendly_address(),
                                signal_data: signal_data.map(hex::encode),
                                proof_of_knowledge: proof_of_knowledge.to_hex(),
                            },
                        ),
                        IncomingStakingTransactionData::UpdateValidator {
                            new_signing_key,
                            new_voting_key,
                            new_reward_address,
                            new_signal_data,
                            new_proof_of_knowledge,
                            proof: _proof,
                        } => PlainTransactionRecipientData::UpdateValidator(
                            PlainUpdateValidatorData {
                                raw: hex::encode(self.recipient_data()),
                                new_signing_key: new_signing_key
                                    .map(|signing_key| signing_key.to_hex()),
                                new_voting_key: new_voting_key
                                    .map(|voting_key| voting_key.to_hex()),
                                new_reward_address: new_reward_address.map(|reward_address| {
                                    reward_address.to_user_friendly_address()
                                }),
                                new_signal_data: new_signal_data
                                    .map(|signal_data| signal_data.map(hex::encode)),
                                new_proof_of_knowledge: new_proof_of_knowledge
                                    .map(|proof_of_knowledge| proof_of_knowledge.to_hex()),
                            },
                        ),
                        IncomingStakingTransactionData::DeactivateValidator {
                            validator_address,
                            proof: _proof,
                        } => {
                            PlainTransactionRecipientData::DeactivateValidator(PlainValidatorData {
                                raw: hex::encode(self.recipient_data()),
                                validator: validator_address.to_user_friendly_address(),
                            })
                        }
                        IncomingStakingTransactionData::ReactivateValidator {
                            validator_address,
                            proof: _proof,
                        } => {
                            PlainTransactionRecipientData::ReactivateValidator(PlainValidatorData {
                                raw: hex::encode(self.recipient_data()),
                                validator: validator_address.to_user_friendly_address(),
                            })
                        }
                        IncomingStakingTransactionData::RetireValidator { proof: _proof } => {
                            PlainTransactionRecipientData::RetireValidator(PlainRawData {
                                raw: hex::encode(self.recipient_data()),
                            })
                        }
                        IncomingStakingTransactionData::SetActiveStake {
                            new_active_balance,
                            proof: _proof,
                        } => {
                            PlainTransactionRecipientData::SetActiveStake(PlainSetActiveStakeData {
                                raw: hex::encode(self.recipient_data()),
                                new_active_balance: new_active_balance.into(),
                            })
                        }
                        IncomingStakingTransactionData::RetireStake {
                            retire_stake,
                            proof: _proof,
                        } => PlainTransactionRecipientData::RetireStake(PlainRetireStakeData {
                            raw: hex::encode(self.recipient_data()),
                            retire_stake: retire_stake.into(),
                        }),
                    }
                    // In the future we might add other staking notifications
                } else if self.inner.recipient_type == AccountType::Vesting {
                    let data = VestingCreationTransactionData::parse(&self.inner).unwrap();
                    PlainTransactionRecipientData::Vesting(PlainVestingData {
                        raw: hex::encode(self.recipient_data()),
                        owner: data.owner.to_user_friendly_address(),
                        start_time: data.start_time,
                        step_amount: data.step_amount.into(),
                        time_step: data.time_step,
                    })
                } else if self.inner.recipient_type == AccountType::HTLC {
                    let data = HtlcCreationTransactionData::parse(&self.inner).unwrap();
                    PlainTransactionRecipientData::Htlc(PlainHtlcData {
                        raw: hex::encode(self.recipient_data()),
                        sender: data.sender.to_user_friendly_address(),
                        recipient: data.recipient.to_user_friendly_address(),
                        hash_algorithm: match data.hash_root {
                            AnyHash::Blake2b(_) => "blake2b".to_string(),
                            AnyHash::Sha256(_) => "sha256".to_string(),
                            AnyHash::Sha512(_) => "sha512".to_string(),
                        },
                        hash_root: data.hash_root.to_hex(),
                        hash_count: data.hash_count,
                        timeout: data.timeout,
                    })
                } else {
                    PlainTransactionRecipientData::Raw(PlainRawData {
                        raw: hex::encode(self.recipient_data()),
                    })
                }
            },
            proof: {
                if self.inner.proof.is_empty() {
                    PlainTransactionProof::Empty(PlainEmptyProof::default())
                } else if self.inner.sender_type == AccountType::HTLC {
                    let proof = OutgoingHTLCTransactionProof::parse(&self.inner).unwrap();
                    match proof {
                        OutgoingHTLCTransactionProof::RegularTransfer {
                            hash_depth,
                            hash_root,
                            pre_image,
                            signature_proof,
                        } => {
                            PlainTransactionProof::RegularTransfer(PlainHtlcRegularTransferProof {
                                raw: hex::encode(self.proof()),
                                hash_algorithm: match hash_root {
                                    AnyHash::Blake2b(_) => "blake2b".to_string(),
                                    AnyHash::Sha256(_) => "sha256".to_string(),
                                    AnyHash::Sha512(_) => "sha512".to_string(),
                                },
                                hash_depth,
                                hash_root: hash_root.to_hex(),
                                pre_image: pre_image.to_hex(),
                                signer: signature_proof.compute_signer().to_user_friendly_address(),
                                signature: match signature_proof.signature {
                                    Signature::Ed25519(ref signature) => signature.to_hex(),
                                    Signature::ES256(ref signature) => signature.to_hex(),
                                },
                                public_key: match signature_proof.public_key {
                                    PublicKey::Ed25519(ref public_key) => public_key.to_hex(),
                                    PublicKey::ES256(ref public_key) => public_key.to_hex(),
                                },
                                path_length: signature_proof.merkle_path.len() as u8,
                            })
                        }
                        OutgoingHTLCTransactionProof::TimeoutResolve {
                            signature_proof_sender,
                        } => PlainTransactionProof::TimeoutResolve(PlainHtlcTimeoutResolveProof {
                            raw: hex::encode(self.proof()),
                            creator: signature_proof_sender
                                .compute_signer()
                                .to_user_friendly_address(),
                            creator_signature: match signature_proof_sender.signature {
                                Signature::Ed25519(ref signature) => signature.to_hex(),
                                Signature::ES256(ref signature) => signature.to_hex(),
                            },
                            creator_public_key: match signature_proof_sender.public_key {
                                PublicKey::Ed25519(ref public_key) => public_key.to_hex(),
                                PublicKey::ES256(ref public_key) => public_key.to_hex(),
                            },
                            creator_path_length: signature_proof_sender.merkle_path.len() as u8,
                        }),
                        OutgoingHTLCTransactionProof::EarlyResolve {
                            signature_proof_recipient,
                            signature_proof_sender,
                        } => PlainTransactionProof::EarlyResolve(PlainHtlcEarlyResolveProof {
                            raw: hex::encode(self.proof()),
                            signer: signature_proof_recipient
                                .compute_signer()
                                .to_user_friendly_address(),
                            signature: match signature_proof_recipient.signature {
                                Signature::Ed25519(ref signature) => signature.to_hex(),
                                Signature::ES256(ref signature) => signature.to_hex(),
                            },
                            public_key: match signature_proof_recipient.public_key {
                                PublicKey::Ed25519(ref public_key) => public_key.to_hex(),
                                PublicKey::ES256(ref public_key) => public_key.to_hex(),
                            },
                            path_length: signature_proof_recipient.merkle_path.len() as u8,
                            creator: signature_proof_sender
                                .compute_signer()
                                .to_user_friendly_address(),
                            creator_signature: match signature_proof_sender.signature {
                                Signature::Ed25519(ref signature) => signature.to_hex(),
                                Signature::ES256(ref signature) => signature.to_hex(),
                            },
                            creator_public_key: match signature_proof_sender.public_key {
                                PublicKey::Ed25519(ref public_key) => public_key.to_hex(),
                                PublicKey::ES256(ref public_key) => public_key.to_hex(),
                            },
                            creator_path_length: signature_proof_sender.merkle_path.len() as u8,
                        }),
                    }
                } else {
                    let proof = SignatureProof::deserialize_all(&self.inner.proof).unwrap();
                    PlainTransactionProof::Standard(PlainStandardProof {
                        raw: hex::encode(self.proof()),
                        signature: match proof.signature {
                            Signature::Ed25519(ref signature) => signature.to_hex(),
                            Signature::ES256(ref signature) => signature.to_hex(),
                        },
                        public_key: match proof.public_key {
                            PublicKey::Ed25519(ref public_key) => public_key.to_hex(),
                            PublicKey::ES256(ref public_key) => public_key.to_hex(),
                        },
                        signer: proof.compute_signer().to_user_friendly_address(),
                        path_length: proof.merkle_path.len() as u8,
                    })
                }
            },
            size: self.serialized_size(),
            valid: self.verify(None).is_ok(),
        }
    }

    pub fn from_plain_transaction(plain: &PlainTransaction) -> Result<Transaction, JsError> {
        let mut tx = Transaction::new(
            &Address::from_string(&plain.sender)?,
            Some(plain.sender_type.into()),
            Some(hex::decode(match plain.sender_data {
                PlainTransactionSenderData::Raw(ref data) => &data.raw,
                PlainTransactionSenderData::RemoveStake(ref data) => &data.raw,
                PlainTransactionSenderData::DeleteValidator(ref data) => &data.raw,
            })?),
            &Address::from_string(&plain.recipient)?,
            Some(plain.recipient_type.into()),
            Some(hex::decode(match plain.data {
                PlainTransactionRecipientData::Raw(ref data) => &data.raw,
                PlainTransactionRecipientData::Vesting(ref data) => &data.raw,
                PlainTransactionRecipientData::Htlc(ref data) => &data.raw,
                PlainTransactionRecipientData::CreateValidator(ref data) => &data.raw,
                PlainTransactionRecipientData::UpdateValidator(ref data) => &data.raw,
                PlainTransactionRecipientData::DeactivateValidator(ref data) => &data.raw,
                PlainTransactionRecipientData::ReactivateValidator(ref data) => &data.raw,
                PlainTransactionRecipientData::RetireValidator(ref data) => &data.raw,
                PlainTransactionRecipientData::CreateStaker(ref data) => &data.raw,
                PlainTransactionRecipientData::AddStake(ref data) => &data.raw,
                PlainTransactionRecipientData::UpdateStaker(ref data) => &data.raw,
                PlainTransactionRecipientData::SetActiveStake(ref data) => &data.raw,
                PlainTransactionRecipientData::RetireStake(ref data) => &data.raw,
            })?),
            plain.value,
            plain.fee,
            Some(plain.flags),
            plain.validity_start_height,
            from_network_id(NetworkId::from_str(&plain.network)?),
        )?;
        tx.set_proof(hex::decode(match plain.proof {
            PlainTransactionProof::Empty(_) => "",
            PlainTransactionProof::Standard(ref data) => &data.raw,
            PlainTransactionProof::RegularTransfer(ref data) => &data.raw,
            PlainTransactionProof::TimeoutResolve(ref data) => &data.raw,
            PlainTransactionProof::EarlyResolve(ref data) => &data.raw,
        })?);

        Ok(tx)
    }
}

/// Enum over all possible meanings of a transaction's sender data.
#[derive(Clone, serde::Serialize, serde::Deserialize, Tsify)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum PlainTransactionSenderData {
    Raw(PlainRawData),
    DeleteValidator(PlainRawData),
    RemoveStake(PlainRawData),
}

/// Enum over all possible meanings of a transaction's recipient data.
#[derive(Clone, serde::Serialize, serde::Deserialize, Tsify)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum PlainTransactionRecipientData {
    Raw(PlainRawData),
    Vesting(PlainVestingData),
    Htlc(PlainHtlcData),
    CreateValidator(PlainCreateValidatorData),
    UpdateValidator(PlainUpdateValidatorData),
    DeactivateValidator(PlainValidatorData),
    ReactivateValidator(PlainValidatorData),
    RetireValidator(PlainRawData),
    CreateStaker(PlainCreateStakerData),
    AddStake(PlainAddStakeData),
    UpdateStaker(PlainUpdateStakerData),
    SetActiveStake(PlainSetActiveStakeData),
    RetireStake(PlainRetireStakeData),
}

/// Placeholder struct to serialize data of transactions as hex strings in the style of the Nimiq 1.0 library.
#[derive(Clone, serde::Serialize, serde::Deserialize, Tsify)]
pub struct PlainRawData {
    pub raw: String,
}

/// JSON-compatible and human-readable format of vesting creation data.
#[derive(Clone, serde::Serialize, serde::Deserialize, Tsify)]
#[serde(rename_all = "camelCase")]
pub struct PlainVestingData {
    pub raw: String,
    pub owner: String,
    pub start_time: u64,
    pub time_step: u64,
    pub step_amount: u64,
}

/// JSON-compatible and human-readable format of HTLC creation data.
#[derive(Clone, serde::Serialize, serde::Deserialize, Tsify)]
#[serde(rename_all = "camelCase")]
pub struct PlainHtlcData {
    pub raw: String,
    pub sender: String,
    pub recipient: String,
    pub hash_algorithm: String,
    pub hash_root: String,
    pub hash_count: u8,
    pub timeout: u64,
}

/// JSON-compatible and human-readable format of validator creation data.
#[derive(Clone, serde::Serialize, serde::Deserialize, Tsify)]
#[serde(rename_all = "camelCase")]
pub struct PlainCreateValidatorData {
    raw: String,
    signing_key: String,
    voting_key: String,
    reward_address: String,
    signal_data: Option<String>,
    proof_of_knowledge: String,
}

/// JSON-compatible and human-readable format of validator update data.
#[derive(Clone, serde::Serialize, serde::Deserialize, Tsify)]
#[serde(rename_all = "camelCase")]
pub struct PlainUpdateValidatorData {
    raw: String,
    new_signing_key: Option<String>,
    new_voting_key: Option<String>,
    new_reward_address: Option<String>,
    new_signal_data: Option<Option<String>>,
    new_proof_of_knowledge: Option<String>,
}

/// JSON-compatible and human-readable format of validator deactivation/reactivation data.
/// Used for DeactivateValidator & ReactivateValidator, as they have the same fields.
#[derive(Clone, serde::Serialize, serde::Deserialize, Tsify)]
#[serde(rename_all = "camelCase")]
pub struct PlainValidatorData {
    pub raw: String,
    pub validator: String,
}

/// JSON-compatible and human-readable format of staker creation data.
#[derive(Clone, serde::Serialize, serde::Deserialize, Tsify)]
#[serde(rename_all = "camelCase")]
pub struct PlainCreateStakerData {
    pub raw: String,
    pub delegation: Option<String>,
}

/// JSON-compatible and human-readable format of add stake data.
#[derive(Clone, serde::Serialize, serde::Deserialize, Tsify)]
#[serde(rename_all = "camelCase")]
pub struct PlainAddStakeData {
    pub raw: String,
    pub staker: String,
}

/// JSON-compatible and human-readable format of update staker data.
#[derive(Clone, serde::Serialize, serde::Deserialize, Tsify)]
#[serde(rename_all = "camelCase")]
pub struct PlainUpdateStakerData {
    pub raw: String,
    pub new_delegation: Option<String>,
    pub reactivate_all_stake: bool,
}

/// JSON-compatible and human-readable format of set active stake data.
#[derive(Clone, serde::Serialize, serde::Deserialize, Tsify)]
#[serde(rename_all = "camelCase")]
pub struct PlainSetActiveStakeData {
    pub raw: String,
    pub new_active_balance: u64,
}

/// JSON-compatible and human-readable format of retire stake data.
#[derive(Clone, serde::Serialize, serde::Deserialize, Tsify)]
#[serde(rename_all = "camelCase")]
pub struct PlainRetireStakeData {
    pub raw: String,
    pub retire_stake: u64,
}

/// Enum over all possible meanings of a transaction's proof.
#[derive(Clone, serde::Serialize, serde::Deserialize, Tsify)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum PlainTransactionProof {
    Empty(PlainEmptyProof),
    Standard(PlainStandardProof),
    RegularTransfer(PlainHtlcRegularTransferProof),
    TimeoutResolve(PlainHtlcTimeoutResolveProof),
    EarlyResolve(PlainHtlcEarlyResolveProof),
}

/// Placeholder struct to serialize an empty proof of transactions
#[derive(Clone, Default, serde::Serialize, serde::Deserialize, Tsify)]
pub struct PlainEmptyProof {
    pub raw: String,
}

/// JSON-compatible and human-readable format of standard transaction proofs.
#[derive(Clone, serde::Serialize, serde::Deserialize, Tsify)]
#[serde(rename_all = "camelCase")]
pub struct PlainStandardProof {
    pub raw: String,
    pub signature: String,
    pub public_key: String,
    pub signer: String,
    pub path_length: u8,
}

/// JSON-compatible and human-readable format of HTLC transfer proofs.
#[derive(Clone, serde::Serialize, serde::Deserialize, Tsify)]
#[serde(rename_all = "camelCase")]
pub struct PlainHtlcRegularTransferProof {
    pub raw: String,
    pub hash_algorithm: String,
    pub hash_depth: u8,
    pub hash_root: String,
    pub pre_image: String,
    /// The signer (also called the "recipient") of the HTLC
    pub signer: String,
    pub signature: String,
    pub public_key: String,
    pub path_length: u8,
}

/// JSON-compatible and human-readable format of HTLC timeout proofs.
#[derive(Clone, serde::Serialize, serde::Deserialize, Tsify)]
#[serde(rename_all = "camelCase")]
pub struct PlainHtlcTimeoutResolveProof {
    pub raw: String,
    /// The creator (also called the "sender") of the HTLC
    pub creator: String,
    pub creator_signature: String,
    pub creator_public_key: String,
    pub creator_path_length: u8,
}

/// JSON-compatible and human-readable format of HTLC early resolve proofs.
#[derive(Clone, serde::Serialize, serde::Deserialize, Tsify)]
#[serde(rename_all = "camelCase")]
pub struct PlainHtlcEarlyResolveProof {
    pub raw: String,
    /// The signer (also called the "recipient") of the HTLC
    pub signer: String,
    pub signature: String,
    pub public_key: String,
    pub path_length: u8,
    /// The creator (also called the "sender") of the HTLC
    pub creator: String,
    pub creator_signature: String,
    pub creator_public_key: String,
    pub creator_path_length: u8,
}

/// JSON-compatible and human-readable format of transactions. E.g. addresses are presented in their human-readable
/// format and address types and the network are represented as strings. Data and proof are serialized as an object
/// describing their contents (not yet implemented, only the `{ raw: string }` fallback is available).
#[derive(Clone, serde::Serialize, serde::Deserialize, Tsify)]
#[serde(rename_all = "camelCase")]
pub struct PlainTransaction {
    /// The transaction's unique hash, used as its identifier. Sometimes also called `txId`.
    pub transaction_hash: String,
    /// The transaction's format. Nimiq transactions can have one of two formats: "basic" and "extended".
    /// Basic transactions are simple value transfers between two regular address types and cannot contain
    /// any extra data. Basic transactions can be serialized to less bytes, so take up less place on the
    /// blockchain. Extended transactions on the other hand are all other transactions: contract creations
    /// and interactions, staking transactions, transactions with exta data, etc.
    #[tsify(type = "PlainTransactionFormat")]
    pub format: TransactionFormat,
    /// The transaction's sender address in human-readable IBAN format.
    pub sender: String,
    /// The type of the transaction's sender. "basic" are regular private-key controlled addresses,
    /// "vesting" and "htlc" are those contract types respectively, and "staking" is the staking contract.
    #[tsify(type = "PlainAccountType")]
    pub sender_type: AccountType,
    /// The transaction's recipient address in human-readable IBAN format.
    pub recipient: String,
    /// The type of the transaction's sender. "basic" are regular private-key controlled addresses,
    /// "vesting" and "htlc" are those contract types respectively, and "staking" is the staking contract.
    #[tsify(type = "PlainAccountType")]
    pub recipient_type: AccountType,
    // The transaction's value in luna (NIM's smallest unit).
    pub value: u64,
    /// The transaction's fee in luna (NIM's smallest unit).
    pub fee: u64,
    /// The transaction's fee-per-byte in luna (NIM's smallest unit).
    pub fee_per_byte: f64,
    /// The block height at which this transaction becomes valid. It is then valid for 7200 blocks (~2 hours).
    pub validity_start_height: u32,
    /// The network name on which this transaction is valid.
    pub network: String,
    /// Any flags that this transaction carries. `0b1 = 1` means it's a contract-creation transaction, `0b10 = 2`
    /// means it's a signalling transaction with 0 value.
    pub flags: u8,
    /// The `sender_data` field serves a purpose based on the transaction's sender type.
    /// It is currently only used for extra information in transactions from the staking contract.
    pub sender_data: PlainTransactionSenderData,
    /// The `data` field of a transaction serves different purposes based on the transaction's recipient type.
    /// For transactions to "basic" address types, this field can contain up to 64 bytes of unstructured data.
    /// For transactions that create contracts or interact with the staking contract, the format of this field
    /// must follow a fixed structure and defines the new contracts' properties or how the staking contract is
    /// changed.
    pub data: PlainTransactionRecipientData,
    /// The `proof` field contains the signature of the eligible signer. The proof field's structure depends on
    /// the transaction's sender type. For transactions from contracts it can also contain additional structured
    /// data before the signature.
    pub proof: PlainTransactionProof,
    /// The transaction's serialized size in bytes. It is used to determine the fee-per-byte that this
    /// transaction pays.
    pub size: usize,
    /// Encodes if the transaction is valid, meaning the signature is valid and the `data` and `proof` fields
    /// follow the correct format for the transaction's recipient and sender type, respectively.
    pub valid: bool,
}

#[cfg(feature = "client")]
impl PlainTransaction {
    pub fn from_reward_event(
        ev: RewardEvent,
        hash: Blake2bHash,
        network: NetworkId,
        block_number: u32,
    ) -> PlainTransaction {
        let size = ev.serialized_size();
        PlainTransaction {
            transaction_hash: hash.to_hex(),
            format: TransactionFormat::Basic, // TODO: bogus: not encoded as transaction on the block chain
            sender: Address::from(Policy::COINBASE_ADDRESS).to_plain(),
            sender_type: AccountType::Basic,
            recipient: Address::from(ev.reward_address).to_plain(),
            recipient_type: AccountType::Basic,
            value: ev.value.into(),
            fee: Coin::ZERO.into(),
            fee_per_byte: 0.0,
            validity_start_height: block_number,
            network: network.to_string().to_lowercase(),
            flags: 0,
            sender_data: PlainTransactionSenderData::Raw(PlainRawData {
                raw: String::from(""),
            }),
            data: PlainTransactionRecipientData::Raw(PlainRawData {
                raw: String::from(""),
            }),
            proof: PlainTransactionProof::Empty(PlainEmptyProof::default()),
            size,
            valid: true,
        }
    }
}

/// Describes the state of a transaction as known by the client.
#[cfg(feature = "client")]
#[derive(Clone, serde::Serialize, serde::Deserialize, Tsify)]
#[serde(rename_all = "lowercase")]
pub enum TransactionState {
    /// The transaction only exists locally and has not been broadcast or accepted by any peers.
    New,
    /// The transaction has been broadcast and accepted by peers and is waiting in the mempool for
    /// inclusion into the blockchain.
    Pending,
    /// The transaction has been included into the blockchain, but not yet finalized by a following
    /// macro block.
    Included,
    /// The transaction is included in the blockchain and has been finalized by a following macro block.
    Confirmed,
    /// The transaction was invalided by a blockchain state change before it could be included into
    /// the chain, or was replaced by a higher-fee transaction, or cannot be applied anymore after a
    /// blockchain rebranch.
    Invalidated,
    /// The transaction's validity window has expired and the transaction can no longer be included into
    /// the blockchain.
    Expired,
}

/// JSON-compatible and human-readable format of transactions, including details about its state in the
/// blockchain. Contains all fields from {@link PlainTransaction}, plus additional fields such as
/// `blockHeight` and `timestamp` if the transaction is included in the blockchain.
#[cfg(feature = "client")]
#[derive(Clone, serde::Deserialize, Tsify)]
#[serde(rename_all = "camelCase")]
pub struct PlainTransactionDetails {
    #[serde(flatten)]
    pub transaction: PlainTransaction,

    pub state: TransactionState,
    #[tsify(optional)]
    pub execution_result: Option<bool>,
    #[tsify(optional)]
    pub block_height: Option<u32>,
    #[tsify(optional)]
    pub confirmations: Option<u32>,
    #[tsify(optional)]
    pub timestamp: Option<u64>,
}

// Manually implement serde::Serialize trait to ensure struct is serialized into a JS Object and not a Map.
//
// Unfortunately, serde cannot serialize a struct that includes a #[serde(flatten)] annotation into an Object,
// and the Github issue for it is closed as "wontfix": https://github.com/serde-rs/serde/issues/1346
#[cfg(feature = "client")]
impl serde::Serialize for PlainTransactionDetails {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut plain = serializer.serialize_struct("PlainTransactionDetails", 22)?;
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
        plain.serialize_field("senderData", &self.transaction.sender_data)?;
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

#[cfg(feature = "client")]
impl PlainTransactionDetails {
    // Construct a TransactionDetails struct with all details manually
    pub fn new(
        tx: &Transaction,
        state: TransactionState,
        execution_result: Option<bool>,
        block_height: Option<u32>,
        timestamp: Option<u64>,
        confirmations: Option<u32>,
    ) -> Self {
        Self {
            transaction: tx.to_plain_transaction(),
            state,
            execution_result,
            block_height,
            timestamp,
            confirmations,
        }
    }
    /// Creates a PlainTransactionDetails struct that can be serialized to JS from a native [HistoricTransaction].
    pub fn try_from_historic_transaction(
        hist_tx: HistoricTransaction,
        current_block: u32,
    ) -> Option<PlainTransactionDetails> {
        let block_number = hist_tx.block_number;
        let block_time = hist_tx.block_time;

        let state = if Policy::last_macro_block(current_block) >= block_number {
            TransactionState::Confirmed
        } else {
            TransactionState::Included
        };

        let (succeeded, transaction) = match hist_tx.data {
            HistoricTransactionData::Basic(ExecutedTransaction::Ok(inner)) => {
                (true, Transaction::from(inner).to_plain_transaction())
            }
            HistoricTransactionData::Basic(ExecutedTransaction::Err(inner)) => {
                (false, Transaction::from(inner).to_plain_transaction())
            }
            HistoricTransactionData::Reward(ref ev) => (
                true,
                PlainTransaction::from_reward_event(
                    ev.clone(),
                    hist_tx.tx_hash().into(),
                    hist_tx.network_id,
                    hist_tx.block_number,
                ),
            ),
            HistoricTransactionData::Penalize(_) => return None,
            HistoricTransactionData::Jail(_) => return None,
            HistoricTransactionData::Equivocation(_) => return None,
        };

        Some(PlainTransactionDetails {
            transaction,
            state,
            execution_result: Some(succeeded),
            block_height: Some(block_number),
            timestamp: Some(block_time),
            confirmations: Some(current_block - block_number + 1),
        })
    }
}

/// JSON-compatible and human-readable format of transaction receipts.
#[cfg(feature = "client")]
#[derive(serde::Serialize, serde::Deserialize, Tsify)]
#[serde(rename_all = "camelCase")]
pub struct PlainTransactionReceipt {
    /// The transaction's unique hash, used as its identifier. Sometimes also called `txId`.
    pub transaction_hash: String,
    /// The transaction's block height where it is included in the blockchain.
    pub block_height: u32,
}

#[cfg(feature = "client")]
impl PlainTransactionReceipt {
    /// Creates a PlainTransactionReceipt struct that can be serialized to JS.
    pub fn from_receipt(receipt: &(Blake2bHash, u32)) -> Self {
        Self {
            transaction_hash: receipt.0.to_hex(),
            block_height: receipt.1,
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

    #[wasm_bindgen(typescript_type = "PlainTransactionReceipt[]")]
    pub type PlainTransactionReceiptArrayType;
}

#[cfg(feature = "primitives")]
#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(typescript_type = "Transaction | PlainTransaction | string | Uint8Array")]
    pub type TransactionAnyType;
}

#[cfg(not(feature = "primitives"))]
#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(typescript_type = "PlainTransaction | string | Uint8Array")]
    pub type TransactionAnyType;
}
