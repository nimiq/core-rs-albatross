use std::str::FromStr;

use nimiq_hash::Blake2bHash;
use nimiq_primitives::{coin::Coin, policy::Policy};
use nimiq_serde::Deserialize;
use nimiq_transaction::account::{
    bridge_contract::{AnyMerkleProof, ChainConfig, OutgoingTransaction},
    htlc_contract::{AnyHash, AnyHash32, AnyHash64},
};
use nimiq_transaction_builder::{Recipient, Sender};
use wasm_bindgen::prelude::*;

use crate::{
    common::{
        address::{Address, OptionalAddress},
        transaction::Transaction,
        utils::to_network_id,
    },
    primitives::{bls_key_pair::BLSKeyPair, public_key::PublicKey},
};

/// The TransactionBuilder class provides helper methods to easily create standard types of transactions.
/// It can only be instantiated from a Client with `client.transactionBuilder()`.
#[wasm_bindgen]
pub struct TransactionBuilder;

#[wasm_bindgen]
impl TransactionBuilder {
    /// Creates a basic transaction that transfers `value` amount of luna (NIM's smallest unit) from the
    /// sender to the recipient.
    ///
    /// The returned transaction is not yet signed. You can sign it e.g. with `tx.sign(keyPair)`.
    ///
    /// Throws when the numbers given for value and fee do not fit within a u64 or the networkId is unknown.
    #[wasm_bindgen(js_name = newBasic)]
    pub fn new_basic(
        sender: &Address,
        recipient: &Address,
        value: u64,
        fee: Option<u64>,
        validity_start_height: u32,
        network_id: u8,
    ) -> Result<Transaction, JsError> {
        let mut builder = nimiq_transaction_builder::TransactionBuilder::new();
        builder
            .with_sender(Sender::new_basic(sender.native_cloned()))
            .with_recipient(Recipient::new_basic(recipient.native_cloned()))
            .with_value(Coin::try_from(value)?)
            .with_fee(Coin::try_from(fee.unwrap_or(0))?)
            .with_validity_start_height(validity_start_height)
            .with_network_id(to_network_id(network_id)?);

        let proof_builder = builder.generate()?;
        let tx = proof_builder.preliminary_transaction().to_owned();
        Ok(Transaction::from(tx))
    }

    /// Creates a basic transaction that transfers `value` amount of luna (NIM's smallest unit) from the
    /// sender to the recipient. It can include arbitrary `data`, up to 64 bytes.
    ///
    /// The returned transaction is not yet signed. You can sign it e.g. with `tx.sign(keyPair)`.
    ///
    /// Throws when the numbers given for value and fee do not fit within a u64 or the networkId is unknown.
    #[wasm_bindgen(js_name = newBasicWithData)]
    pub fn new_basic_with_data(
        sender: &Address,
        recipient: &Address,
        data: Vec<u8>,
        value: u64,
        fee: Option<u64>,
        validity_start_height: u32,
        network_id: u8,
    ) -> Result<Transaction, JsError> {
        let mut builder = nimiq_transaction_builder::TransactionBuilder::new();
        builder
            .with_sender(Sender::new_basic(sender.native_cloned()))
            .with_recipient(Recipient::new_basic_with_data(
                recipient.native_cloned(),
                data,
            ))
            .with_value(Coin::try_from(value)?)
            .with_fee(Coin::try_from(fee.unwrap_or(0))?)
            .with_validity_start_height(validity_start_height)
            .with_network_id(to_network_id(network_id)?);

        let proof_builder = builder.generate()?;
        let tx = proof_builder.preliminary_transaction().to_owned();
        Ok(Transaction::from(tx))
    }

    // pub fn new_create_vesting()

    // pub fn new_redeem_vesting()

    // pub fn new_create_htlc()

    // pub fn new_redeem_htlc()

    // pub fn new_refund_htlc()

    // pub fn new_redeem_htlc_early()

    // pub fn sign_htlc_early()

    /// Creates a new staker in the staking contract and transfers `value` amount of luna (NIM's smallest unit)
    /// from the sender account to this new staker.
    ///
    /// The returned transaction is not yet signed. You can sign it e.g. with `tx.sign(keyPair)`.
    ///
    /// Throws when the numbers given for value and fee do not fit within a u64 or the networkId is unknown.
    #[wasm_bindgen(js_name = newCreateStaker)]
    pub fn new_create_staker(
        sender: &Address,
        delegation: &OptionalAddress,
        value: u64,
        fee: Option<u64>,
        validity_start_height: u32,
        network_id: u8,
    ) -> Result<Transaction, JsError> {
        let delegation = wasm_bindgen_derive::try_from_js_option::<Address>(delegation)
            .map_err(|err| JsError::new(&err))?;

        let mut recipient = Recipient::new_staking_builder();
        recipient.create_staker(delegation.map(|addr| addr.take_native()));

        let mut builder = nimiq_transaction_builder::TransactionBuilder::new();
        builder
            .with_sender(Sender::new_basic(sender.native_cloned()))
            .with_recipient(recipient.generate().unwrap())
            .with_value(Coin::try_from(value)?)
            .with_fee(Coin::try_from(fee.unwrap_or(0))?)
            .with_validity_start_height(validity_start_height)
            .with_network_id(to_network_id(network_id)?);

        let proof_builder = builder.generate()?;
        let tx = proof_builder.preliminary_transaction().to_owned();
        Ok(Transaction::from(tx))
    }

    /// Adds stake to a staker in the staking contract and transfers `value` amount of luna (NIM's smallest unit)
    /// from the sender account to this staker.
    ///
    /// The returned transaction is not yet signed. You can sign it e.g. with `tx.sign(keyPair)`.
    ///
    /// Throws when the numbers given for value and fee do not fit within a u64 or the networkId is unknown.
    #[wasm_bindgen(js_name = newAddStake)]
    pub fn new_add_stake(
        sender: &Address,
        staker_address: &Address,
        value: u64,
        fee: Option<u64>,
        validity_start_height: u32,
        network_id: u8,
    ) -> Result<Transaction, JsError> {
        let mut recipient = Recipient::new_staking_builder();
        recipient.stake(staker_address.native_cloned());

        let mut builder = nimiq_transaction_builder::TransactionBuilder::new();
        builder
            .with_sender(Sender::new_basic(sender.native_cloned()))
            .with_recipient(recipient.generate().unwrap())
            .with_value(Coin::try_from(value)?)
            .with_fee(Coin::try_from(fee.unwrap_or(0))?)
            .with_validity_start_height(validity_start_height)
            .with_network_id(to_network_id(network_id)?);

        let proof_builder = builder.generate()?;
        let tx = proof_builder.preliminary_transaction().to_owned();
        Ok(Transaction::from(tx))
    }

    /// Updates a staker in the staking contract to stake for a different validator. This is a
    /// signaling transaction and as such does not transfer any value.
    ///
    /// The returned transaction is not yet signed. You can sign it e.g. with `tx.sign(keyPair)`.
    ///
    /// Throws when the number given for fee does not fit within a u64 or the networkId is unknown.
    #[wasm_bindgen(js_name = newUpdateStaker)]
    pub fn new_update_staker(
        sender: &Address,
        new_delegation: &OptionalAddress,
        reactivate_all_stake: bool,
        fee: Option<u64>,
        validity_start_height: u32,
        network_id: u8,
    ) -> Result<Transaction, JsError> {
        let new_delegation = wasm_bindgen_derive::try_from_js_option::<Address>(new_delegation)
            .map_err(|err| JsError::new(&err))?;

        let mut recipient = Recipient::new_staking_builder();
        recipient.update_staker(
            new_delegation.map(|addr| addr.take_native()),
            reactivate_all_stake,
        );

        let mut builder = nimiq_transaction_builder::TransactionBuilder::new();
        builder
            .with_sender(Sender::new_basic(sender.native_cloned()))
            .with_recipient(recipient.generate().unwrap())
            .with_value(Coin::ZERO)
            .with_fee(Coin::try_from(fee.unwrap_or(0))?)
            .with_validity_start_height(validity_start_height)
            .with_network_id(to_network_id(network_id)?);

        let proof_builder = builder.generate()?;
        let tx = proof_builder.preliminary_transaction().to_owned();
        Ok(Transaction::from(tx))
    }

    /// Sets the active stake balance of the staker. This is a
    /// signaling transaction and as such does not transfer any value.
    ///
    /// The returned transaction is not yet signed. You can sign it e.g. with `tx.sign(keyPair)`.
    ///
    /// Throws when the numbers given for fee and `new_active_balance` do not fit within a u64 or the networkId is unknown.
    #[wasm_bindgen(js_name = newSetActiveStake)]
    pub fn new_set_active_stake(
        sender: &Address,
        new_active_balance: u64,
        fee: Option<u64>,
        validity_start_height: u32,
        network_id: u8,
    ) -> Result<Transaction, JsError> {
        let mut recipient = Recipient::new_staking_builder();
        recipient.set_active_stake(Coin::try_from(new_active_balance)?);

        let mut builder = nimiq_transaction_builder::TransactionBuilder::new();
        builder
            .with_sender(Sender::new_basic(sender.native_cloned()))
            .with_recipient(recipient.generate().unwrap())
            .with_value(Coin::ZERO)
            .with_fee(Coin::try_from(fee.unwrap_or(0))?)
            .with_validity_start_height(validity_start_height)
            .with_network_id(to_network_id(network_id)?);

        let proof_builder = builder.generate()?;
        let tx = proof_builder.preliminary_transaction().to_owned();
        Ok(Transaction::from(tx))
    }

    /// Retires a portion of the inactive stake balance of the staker. This is a
    /// signaling transaction and as such does not transfer any value.
    ///
    /// The returned transaction is not yet signed. You can sign it e.g. with `tx.sign(keyPair)`.
    ///
    /// Throws when the numbers given for fee and `retire_stake` do not fit within a u64 or the networkId is unknown.
    #[wasm_bindgen(js_name = newRetireStake)]
    pub fn retire_stake(
        sender: &Address,
        retire_stake: u64,
        fee: Option<u64>,
        validity_start_height: u32,
        network_id: u8,
    ) -> Result<Transaction, JsError> {
        let mut recipient = Recipient::new_staking_builder();
        recipient.retire_stake(Coin::try_from(retire_stake)?);

        let mut builder = nimiq_transaction_builder::TransactionBuilder::new();
        builder
            .with_sender(Sender::new_basic(sender.native_cloned()))
            .with_recipient(recipient.generate().unwrap())
            .with_value(Coin::ZERO)
            .with_fee(Coin::try_from(fee.unwrap_or(0))?)
            .with_validity_start_height(validity_start_height)
            .with_network_id(to_network_id(network_id)?);

        let proof_builder = builder.generate()?;
        let tx = proof_builder.preliminary_transaction().to_owned();
        Ok(Transaction::from(tx))
    }

    /// Removes stake from the staking contract and transfers `value` amount of luna (NIM's smallest unit)
    /// from the staker to the recipient.
    ///
    /// The returned transaction is not yet signed. You can sign it e.g. with `tx.sign(keyPair)`.
    ///
    /// Throws when the numbers given for value and fee do not fit within a u64 or the networkId is unknown.
    #[wasm_bindgen(js_name = newRemoveStake)]
    pub fn new_remove_stake(
        recipient: &Address,
        value: u64,
        fee: Option<u64>,
        validity_start_height: u32,
        network_id: u8,
    ) -> Result<Transaction, JsError> {
        let sender = Sender::new_staking_builder()
            .remove_stake()
            .generate()
            .unwrap();
        let recipient = Recipient::new_basic(recipient.native_cloned());

        let mut builder = nimiq_transaction_builder::TransactionBuilder::new();
        builder
            .with_sender(sender)
            .with_recipient(recipient)
            .with_value(Coin::try_from(value)?)
            .with_fee(Coin::try_from(fee.unwrap_or(0))?)
            .with_validity_start_height(validity_start_height)
            .with_network_id(to_network_id(network_id)?);

        let proof_builder = builder.generate()?;
        let tx = proof_builder.preliminary_transaction().to_owned();
        Ok(Transaction::from(tx))
    }

    /// Registers a new validator in the staking contract.
    ///
    /// The returned transaction is not yet signed. You can sign it e.g. with `tx.sign(keyPair)`.
    ///
    /// Throws when the fee does not fit within a u64 or the `networkId` is unknown.
    #[wasm_bindgen(js_name = newCreateValidator)]
    pub fn new_create_validator(
        sender: &Address,
        reward_address: &Address,
        signing_key: &PublicKey,
        voting_key_pair: &BLSKeyPair,
        signal_data: Option<String>,
        fee: Option<u64>,
        validity_start_height: u32,
        network_id: u8,
    ) -> Result<Transaction, JsError> {
        let native_signal_data = signal_data
            .map(|r| Blake2bHash::from_str(&r))
            .transpose()
            .map_err(|e| JsError::new(&e.to_string()))?;
        let mut recipient = Recipient::new_staking_builder();
        recipient.create_validator(
            *signing_key.native_ref(),
            voting_key_pair.native_ref(),
            reward_address.native_cloned(),
            native_signal_data,
        );

        let mut builder = nimiq_transaction_builder::TransactionBuilder::new();
        builder
            .with_sender(Sender::new_basic(sender.native_cloned()))
            .with_recipient(recipient.generate().unwrap())
            .with_value(Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT))
            .with_fee(Coin::try_from(fee.unwrap_or(0))?)
            .with_validity_start_height(validity_start_height)
            .with_network_id(to_network_id(network_id)?);

        let proof_builder = builder.generate()?;
        let tx = proof_builder.preliminary_transaction().to_owned();
        Ok(Transaction::from(tx))
    }

    /// Updates parameters of a validator in the staking contract.
    ///
    /// The returned transaction is not yet signed. You can sign it e.g. with `tx.sign(keyPair)`.
    ///
    /// Throws when the fee does not fit within a u64 or the `networkId` is unknown.
    #[wasm_bindgen(js_name = newUpdateValidator)]
    pub fn new_update_validator(
        sender: &Address,
        reward_address: Option<Address>,
        signing_key: Option<PublicKey>,
        voting_key_pair: Option<BLSKeyPair>,
        signal_data: Option<String>,
        fee: Option<u64>,
        validity_start_height: u32,
        network_id: u8,
    ) -> Result<Transaction, JsError> {
        let native_signal_data = signal_data
            .map(|r| Blake2bHash::from_str(&r))
            .transpose()
            .map_err(|e| JsError::new(&e.to_string()))?
            .map(Some);
        let mut recipient = Recipient::new_staking_builder();
        let native_signing_key = signing_key.map(|r| *r.native_ref());
        let native_voting_key_pair = voting_key_pair.map(|r| r.native_ref().clone());
        let native_reward_address = reward_address.map(|r| r.native_cloned());

        recipient.update_validator(
            native_signing_key,
            native_voting_key_pair.as_ref(),
            native_reward_address,
            native_signal_data,
        );

        let mut builder = nimiq_transaction_builder::TransactionBuilder::new();
        builder
            .with_sender(Sender::new_basic(sender.native_cloned()))
            .with_recipient(recipient.generate().unwrap())
            .with_value(Coin::ZERO)
            .with_fee(Coin::try_from(fee.unwrap_or(0))?)
            .with_validity_start_height(validity_start_height)
            .with_network_id(to_network_id(network_id)?);

        let proof_builder = builder.generate()?;
        let tx = proof_builder.preliminary_transaction().to_owned();
        Ok(Transaction::from(tx))
    }

    /// Deactivates a validator in the staking contract.
    ///
    /// The returned transaction is not yet signed. You can sign it e.g. with `tx.sign(keyPair)`.
    ///
    /// Throws when the fee does not fit within a u64 or the `networkId` is unknown.
    #[wasm_bindgen(js_name = newDeactivateValidator)]
    pub fn new_deactivate_validator(
        sender: &Address,
        validator: &Address,
        fee: Option<u64>,
        validity_start_height: u32,
        network_id: u8,
    ) -> Result<Transaction, JsError> {
        let mut recipient = Recipient::new_staking_builder();
        recipient.deactivate_validator(validator.native_cloned());

        let mut builder = nimiq_transaction_builder::TransactionBuilder::new();
        builder
            .with_sender(Sender::new_basic(sender.native_cloned()))
            .with_recipient(recipient.generate().unwrap())
            .with_value(Coin::ZERO)
            .with_fee(Coin::try_from(fee.unwrap_or(0))?)
            .with_validity_start_height(validity_start_height)
            .with_network_id(to_network_id(network_id)?);

        let proof_builder = builder.generate()?;
        let tx = proof_builder.preliminary_transaction().to_owned();
        Ok(Transaction::from(tx))
    }

    /// Sets the signal data of a validator in the staking contract. In contrast to
    /// `newUpdateValidator`, this transaction is signed with the validator's *signing (warm) key*,
    /// so the cold key is not required to signal protocol upgrades. Pass `undefined` as
    /// `signalData` to clear the signal.
    ///
    /// The returned transaction is not yet signed. You can sign it e.g. with `tx.sign(keyPair)`.
    ///
    /// Throws when the fee does not fit within a u64 or the `networkId` is unknown.
    #[wasm_bindgen(js_name = newSetSignalData)]
    pub fn new_set_signal_data(
        sender: &Address,
        validator: &Address,
        signal_data: Option<String>,
        fee: Option<u64>,
        validity_start_height: u32,
        network_id: u8,
    ) -> Result<Transaction, JsError> {
        let native_signal_data = signal_data
            .map(|r| Blake2bHash::from_str(&r))
            .transpose()
            .map_err(|e| JsError::new(&e.to_string()))?;
        let mut recipient = Recipient::new_staking_builder();
        recipient.set_signal_data(validator.native_cloned(), native_signal_data);

        let mut builder = nimiq_transaction_builder::TransactionBuilder::new();
        builder
            .with_sender(Sender::new_basic(sender.native_cloned()))
            .with_recipient(recipient.generate().unwrap())
            .with_value(Coin::ZERO)
            .with_fee(Coin::try_from(fee.unwrap_or(0))?)
            .with_validity_start_height(validity_start_height)
            .with_network_id(to_network_id(network_id)?);

        let proof_builder = builder.generate()?;
        let tx = proof_builder.preliminary_transaction().to_owned();
        Ok(Transaction::from(tx))
    }

    /// Signals support for the given protocol `version` with the validator's *signing (warm) key*
    /// by updating the validator's signal data in the staking contract. In contrast to
    /// `newSetSignalData`, this only updates the protocol-version bytes of the signal data and
    /// preserves the rest. To clear the signal data entirely, use `newSetSignalData` with `null`.
    ///
    /// The returned transaction is not yet signed. You can sign it e.g. with `tx.sign(keyPair)`.
    ///
    /// Throws when the fee does not fit within a u64 or the `networkId` is unknown.
    #[wasm_bindgen(js_name = newSignalVersion)]
    pub fn new_signal_version(
        sender: &Address,
        validator: &Address,
        version: u16,
        fee: Option<u64>,
        validity_start_height: u32,
        network_id: u8,
    ) -> Result<Transaction, JsError> {
        let mut recipient = Recipient::new_staking_builder();
        recipient.signal_version(validator.native_cloned(), version);

        let mut builder = nimiq_transaction_builder::TransactionBuilder::new();
        builder
            .with_sender(Sender::new_basic(sender.native_cloned()))
            .with_recipient(recipient.generate().unwrap())
            .with_value(Coin::ZERO)
            .with_fee(Coin::try_from(fee.unwrap_or(0))?)
            .with_validity_start_height(validity_start_height)
            .with_network_id(to_network_id(network_id)?);

        let proof_builder = builder.generate()?;
        let tx = proof_builder.preliminary_transaction().to_owned();
        Ok(Transaction::from(tx))
    }

    // pub fn new_reactivate_validator()

    /// Deleted a validator the staking contract. The deposit is returned to the Sender
    ///
    /// The returned transaction is not yet signed. You can sign it e.g. with `tx.sign(keyPair)`.
    ///
    /// Throws when the fee does not fit within a u64 or the `networkId` is unknown.
    #[wasm_bindgen(js_name = newDeleteValidator)]
    pub fn new_delete_validator(
        sender: &Address,
        fee: Option<u64>,
        validity_start_height: u32,
        network_id: u8,
    ) -> Result<Transaction, JsError> {
        let recipient = Recipient::new_basic(sender.native_cloned());

        let mut builder = nimiq_transaction_builder::TransactionBuilder::new();
        builder
            .with_sender(
                Sender::new_staking_builder()
                    .delete_validator()
                    .generate()
                    .unwrap(),
            )
            .with_recipient(recipient)
            .with_value(Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT))
            .with_fee(Coin::try_from(fee.unwrap_or(0))?)
            .with_validity_start_height(validity_start_height)
            .with_network_id(to_network_id(network_id)?);

        let proof_builder = builder.generate()?;
        let tx = proof_builder.preliminary_transaction().to_owned();
        Ok(Transaction::from(tx))
    }

    /// Retires a validator in the staking contract.
    ///
    /// The returned transaction is not yet signed. You can sign it e.g. with `tx.sign(keyPair)`.
    ///
    /// Throws when the fee does not fit within a u64 or the `networkId` is unknown.
    #[wasm_bindgen(js_name = newRetireValidator)]
    pub fn new_retire_validator(
        sender: &Address,
        fee: Option<u64>,
        validity_start_height: u32,
        network_id: u8,
    ) -> Result<Transaction, JsError> {
        let mut recipient = Recipient::new_staking_builder();
        recipient.retire_validator();

        let mut builder = nimiq_transaction_builder::TransactionBuilder::new();
        builder
            .with_sender(Sender::new_basic(sender.native_cloned()))
            .with_recipient(recipient.generate().unwrap())
            .with_value(Coin::ZERO)
            .with_fee(Coin::try_from(fee.unwrap_or(0))?)
            .with_validity_start_height(validity_start_height)
            .with_network_id(to_network_id(network_id)?);

        let proof_builder = builder.generate()?;
        let tx = proof_builder.preliminary_transaction().to_owned();
        Ok(Transaction::from(tx))
    }

    /// Creates a new oracle contract owned by `owner` that stores up to `hash_count` hashes, and
    /// transfers `value` amount of luna (NIM's smallest unit) from the sender to it as its deposit.
    ///
    /// The returned transaction is not yet signed. You can sign it e.g. with `tx.sign(keyPair)`.
    ///
    /// Throws when `hash_count` is zero, the numbers given for value and fee do not fit within a u64
    /// or the networkId is unknown.
    #[wasm_bindgen(js_name = newCreateOracle)]
    pub fn new_create_oracle(
        sender: &Address,
        owner: &Address,
        hash_count: u16,
        value: u64,
        fee: Option<u64>,
        validity_start_height: u32,
        network_id: u8,
    ) -> Result<Transaction, JsError> {
        let mut recipient = Recipient::new_oracle_builder(owner.native_cloned());
        recipient.with_hash_count(hash_count);

        let mut builder = nimiq_transaction_builder::TransactionBuilder::new();
        builder
            .with_sender(Sender::new_basic(sender.native_cloned()))
            .with_recipient(recipient.generate()?)
            .with_value(Coin::try_from(value)?)
            .with_fee(Coin::try_from(fee.unwrap_or(0))?)
            .with_validity_start_height(validity_start_height)
            .with_network_id(to_network_id(network_id)?);

        let proof_builder = builder.generate()?;
        let tx = proof_builder.preliminary_transaction().to_owned();
        Ok(Transaction::from(tx))
    }

    /// Appends `hashes` to an oracle contract. The hashes are given as hex strings and must all use
    /// the oracle's `hash_algorithm`, one of `blake2b`, `sha256`, `sha512` or `keccak256`. This is a
    /// signaling transaction and as such does not transfer any value.
    ///
    /// The returned transaction is not yet signed. Sign it with the key pair of the sender and of
    /// the oracle owner, e.g. with `tx.sign(senderKeyPair, ownerKeyPair)`.
    ///
    /// Throws when a hash cannot be parsed, the number given for fee does not fit within a u64 or
    /// the networkId is unknown.
    #[wasm_bindgen(js_name = newUpdateOracle)]
    pub fn new_update_oracle(
        sender: &Address,
        oracle: &Address,
        hash_algorithm: &str,
        hashes: Vec<String>,
        fee: Option<u64>,
        validity_start_height: u32,
        network_id: u8,
    ) -> Result<Transaction, JsError> {
        let hashes = hashes
            .iter()
            .map(|hash| parse_hash(hash_algorithm, hash))
            .collect::<Result<Vec<_>, _>>()?;

        let mut builder = nimiq_transaction_builder::TransactionBuilder::new();
        builder
            .with_sender(Sender::new_basic(sender.native_cloned()))
            .with_recipient(Recipient::new_oracle_update(oracle.native_cloned(), hashes))
            .with_value(Coin::ZERO)
            .with_fee(Coin::try_from(fee.unwrap_or(0))?)
            .with_validity_start_height(validity_start_height)
            .with_network_id(to_network_id(network_id)?);

        let proof_builder = builder.generate()?;
        let tx = proof_builder.preliminary_transaction().to_owned();
        Ok(Transaction::from(tx))
    }

    /// Transfers the ownership of an oracle contract to `new_owner`. This is a signaling
    /// transaction and as such does not transfer any value.
    ///
    /// The returned transaction is not yet signed. Sign it with the key pair of the sender and of
    /// the current oracle owner, e.g. with `tx.sign(senderKeyPair, ownerKeyPair)`.
    ///
    /// Throws when the number given for fee does not fit within a u64 or the networkId is unknown.
    #[wasm_bindgen(js_name = newChangeOracleOwner)]
    pub fn new_change_oracle_owner(
        sender: &Address,
        oracle: &Address,
        new_owner: &Address,
        fee: Option<u64>,
        validity_start_height: u32,
        network_id: u8,
    ) -> Result<Transaction, JsError> {
        let mut builder = nimiq_transaction_builder::TransactionBuilder::new();
        builder
            .with_sender(Sender::new_basic(sender.native_cloned()))
            .with_recipient(Recipient::new_oracle_change_owner(
                oracle.native_cloned(),
                new_owner.native_cloned(),
            ))
            .with_value(Coin::ZERO)
            .with_fee(Coin::try_from(fee.unwrap_or(0))?)
            .with_validity_start_height(validity_start_height)
            .with_network_id(to_network_id(network_id)?);

        let proof_builder = builder.generate()?;
        let tx = proof_builder.preliminary_transaction().to_owned();
        Ok(Transaction::from(tx))
    }

    /// Withdraws the deposit of an oracle contract to `recipient`, which deletes the contract.
    /// `value` must equal the full balance of the contract. The contract does not accept a fee on
    /// this transaction, so none is set.
    ///
    /// The returned transaction is not yet signed. Sign it with the key pair of the oracle owner,
    /// e.g. with `tx.sign(ownerKeyPair)`.
    ///
    /// Throws when the number given for value does not fit within a u64 or the networkId is unknown.
    #[wasm_bindgen(js_name = newDeleteOracle)]
    pub fn new_delete_oracle(
        oracle: &Address,
        recipient: &Address,
        value: u64,
        validity_start_height: u32,
        network_id: u8,
    ) -> Result<Transaction, JsError> {
        let mut builder = nimiq_transaction_builder::TransactionBuilder::new();
        builder
            .with_sender(Sender::new_oracle(oracle.native_cloned()))
            .with_recipient(Recipient::new_basic(recipient.native_cloned()))
            .with_value(Coin::try_from(value)?)
            .with_fee(Coin::ZERO)
            .with_validity_start_height(validity_start_height)
            .with_network_id(to_network_id(network_id)?);

        let proof_builder = builder.generate()?;
        let tx = proof_builder.preliminary_transaction().to_owned();
        Ok(Transaction::from(tx))
    }

    /// Creates a new bridge contract owned by `owner` for the source chain `source_chain_id`, and
    /// transfers `value` amount of luna (NIM's smallest unit) from the sender to it as its initial
    /// balance. `chain_config` is the serialized `ChainConfig` of the source chain, and `oracle` the
    /// oracle contract whose states the bridge verifies burn proofs against.
    ///
    /// The returned transaction is not yet signed. You can sign it e.g. with `tx.sign(keyPair)`.
    ///
    /// Throws when the chain config cannot be deserialized, the numbers given for value and fee do
    /// not fit within a u64 or the networkId is unknown.
    #[wasm_bindgen(js_name = newCreateBridge)]
    pub fn new_create_bridge(
        sender: &Address,
        owner: &Address,
        oracle: &Address,
        source_chain_id: u32,
        chain_config: &[u8],
        value: u64,
        fee: Option<u64>,
        validity_start_height: u32,
        network_id: u8,
    ) -> Result<Transaction, JsError> {
        let mut recipient = Recipient::new_bridge_builder();
        recipient
            .with_owner(owner.native_cloned())
            .with_oracle_address(oracle.native_cloned())
            .with_source_chain_id(source_chain_id)
            .with_chain_config(ChainConfig::deserialize_all(chain_config)?);

        let mut builder = nimiq_transaction_builder::TransactionBuilder::new();
        builder
            .with_sender(Sender::new_basic(sender.native_cloned()))
            .with_recipient(recipient.generate()?)
            .with_value(Coin::try_from(value)?)
            .with_fee(Coin::try_from(fee.unwrap_or(0))?)
            .with_validity_start_height(validity_start_height)
            .with_network_id(to_network_id(network_id)?);

        let proof_builder = builder.generate()?;
        let tx = proof_builder.preliminary_transaction().to_owned();
        Ok(Transaction::from(tx))
    }

    /// Deposits `value` amount of luna (NIM's smallest unit) from the sender into a bridge contract,
    /// locking it for transfer to the bridge's destination chain. The bridge contract does not
    /// interpret `data`; the bridge relayer reads the destination of the deposit from it.
    ///
    /// The returned transaction is not yet signed. You can sign it e.g. with `tx.sign(keyPair)`.
    ///
    /// Throws when the numbers given for value and fee do not fit within a u64 or the networkId is unknown.
    #[wasm_bindgen(js_name = newBridgeDeposit)]
    pub fn new_bridge_deposit(
        sender: &Address,
        bridge: &Address,
        data: Vec<u8>,
        value: u64,
        fee: Option<u64>,
        validity_start_height: u32,
        network_id: u8,
    ) -> Result<Transaction, JsError> {
        let mut builder = nimiq_transaction_builder::TransactionBuilder::new();
        builder
            .with_sender(Sender::new_basic(sender.native_cloned()))
            .with_recipient(Recipient::new_bridge_deposit(bridge.native_cloned(), data))
            .with_value(Coin::try_from(value)?)
            .with_fee(Coin::try_from(fee.unwrap_or(0))?)
            .with_validity_start_height(validity_start_height)
            .with_network_id(to_network_id(network_id)?);

        let proof_builder = builder.generate()?;
        let tx = proof_builder.preliminary_transaction().to_owned();
        Ok(Transaction::from(tx))
    }

    /// Releases `value` amount of luna (NIM's smallest unit) from a bridge contract to `recipient`,
    /// against a proof that the corresponding tokens were burned on the source chain. `value` and
    /// `recipient` must match the burn transaction. `merkle_proof` is the serialized
    /// `AnyMerkleProof` of the burn transaction, and `oracle_state_index` the index of the oracle
    /// state it is verified against.
    ///
    /// The returned transaction is not yet signed. Anyone can sign it, e.g. with `tx.sign(keyPair)`;
    /// the fee is paid by the account of that key pair, even if the release fails.
    ///
    /// Throws when the burn proof is invalid, the numbers given for value and fee do not fit within
    /// a u64 or the networkId is unknown.
    #[wasm_bindgen(js_name = newBridgeRelease)]
    pub fn new_bridge_release(
        bridge: &Address,
        recipient: &Address,
        burn_transaction_data: Vec<u8>,
        merkle_proof: &[u8],
        oracle_state_index: u64,
        value: u64,
        fee: Option<u64>,
        validity_start_height: u32,
        network_id: u8,
    ) -> Result<Transaction, JsError> {
        let burn_proof = OutgoingTransaction::new(
            burn_transaction_data,
            AnyMerkleProof::deserialize_all(merkle_proof)?,
            oracle_state_index,
        )?;

        let mut builder = nimiq_transaction_builder::TransactionBuilder::new();
        builder
            .with_sender(Sender::new_bridge(bridge.native_cloned(), burn_proof))
            .with_recipient(Recipient::new_basic(recipient.native_cloned()))
            .with_value(Coin::try_from(value)?)
            .with_fee(Coin::try_from(fee.unwrap_or(0))?)
            .with_validity_start_height(validity_start_height)
            .with_network_id(to_network_id(network_id)?);

        let proof_builder = builder.generate()?;
        let tx = proof_builder.preliminary_transaction().to_owned();
        Ok(Transaction::from(tx))
    }
}

/// Parses a hex-encoded hash of the given algorithm.
fn parse_hash(hash_algorithm: &str, hash: &str) -> Result<AnyHash, JsError> {
    Ok(match hash_algorithm {
        "blake2b" => AnyHash::Blake2b(AnyHash32::from_str(hash)?),
        "sha256" => AnyHash::Sha256(AnyHash32::from_str(hash)?),
        "sha512" => AnyHash::Sha512(AnyHash64::from_str(hash)?),
        "keccak256" => AnyHash::Keccak256(AnyHash32::from_str(hash)?),
        _ => {
            return Err(JsError::new(&format!(
                "Unknown hash algorithm: {hash_algorithm}"
            )))
        }
    })
}
