use nimiq_primitives::{coin::Coin, policy::Policy};
use nimiq_transaction_builder::{Recipient, Sender};
use wasm_bindgen::prelude::*;

use crate::{
    address::Address,
    primitives::{bls_key_pair::BLSKeyPair, public_key::PublicKey},
    transaction::Transaction,
    utils::to_network_id,
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
            .with_sender(Sender::new_basic(sender.native_ref().clone()))
            .with_recipient(Recipient::new_basic(recipient.native_ref().clone()))
            .with_value(Coin::try_from(value)?)
            .with_fee(Coin::try_from(fee.unwrap_or(0))?)
            .with_validity_start_height(validity_start_height)
            .with_network_id(to_network_id(network_id)?);

        let proof_builder = builder.generate()?;
        let tx = proof_builder.preliminary_transaction().to_owned();
        Ok(Transaction::from_native(tx))
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
            .with_sender(Sender::new_basic(sender.native_ref().clone()))
            .with_recipient(Recipient::new_basic_with_data(
                recipient.native_ref().clone(),
                data,
            ))
            .with_value(Coin::try_from(value)?)
            .with_fee(Coin::try_from(fee.unwrap_or(0))?)
            .with_validity_start_height(validity_start_height)
            .with_network_id(to_network_id(network_id)?);

        let proof_builder = builder.generate()?;
        let tx = proof_builder.preliminary_transaction().to_owned();
        Ok(Transaction::from_native(tx))
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
        delegation: &Address,
        value: u64,
        fee: Option<u64>,
        validity_start_height: u32,
        network_id: u8,
    ) -> Result<Transaction, JsError> {
        let mut recipient = Recipient::new_staking_builder();
        recipient.create_staker(Some(delegation.native_ref().clone()));

        let mut builder = nimiq_transaction_builder::TransactionBuilder::new();
        builder
            .with_sender(Sender::new_basic(sender.native_ref().clone()))
            .with_recipient(recipient.generate().unwrap())
            .with_value(Coin::try_from(value)?)
            .with_fee(Coin::try_from(fee.unwrap_or(0))?)
            .with_validity_start_height(validity_start_height)
            .with_network_id(to_network_id(network_id)?);

        let proof_builder = builder.generate()?;
        let tx = proof_builder.preliminary_transaction().to_owned();
        Ok(Transaction::from_native(tx))
    }

    /// Adds stake to a staker in the staking contract and transfers `value` amount of luna (NIM's smallest unit)
    /// from the sender account to this staker.
    ///
    /// The returned transaction is not yet signed. You can sign it e.g. with `tx.sign(keyPair)`.
    ///
    /// Throws when the numbers given for value and fee do not fit within a u64 or the networkId is unknown.
    #[wasm_bindgen(js_name = newStake)]
    pub fn new_stake(
        sender: &Address,
        staker_address: &Address,
        value: u64,
        fee: Option<u64>,
        validity_start_height: u32,
        network_id: u8,
    ) -> Result<Transaction, JsError> {
        let mut recipient = Recipient::new_staking_builder();
        recipient.stake(staker_address.native_ref().clone());

        let mut builder = nimiq_transaction_builder::TransactionBuilder::new();
        builder
            .with_sender(Sender::new_basic(sender.native_ref().clone()))
            .with_recipient(recipient.generate().unwrap())
            .with_value(Coin::try_from(value)?)
            .with_fee(Coin::try_from(fee.unwrap_or(0))?)
            .with_validity_start_height(validity_start_height)
            .with_network_id(to_network_id(network_id)?);

        let proof_builder = builder.generate()?;
        let tx = proof_builder.preliminary_transaction().to_owned();
        Ok(Transaction::from_native(tx))
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
        new_delegation: &Address,
        reactivate_all_stake: bool,
        fee: Option<u64>,
        validity_start_height: u32,
        network_id: u8,
    ) -> Result<Transaction, JsError> {
        let mut recipient = Recipient::new_staking_builder();
        recipient.update_staker(
            Some(new_delegation.native_ref().clone()),
            reactivate_all_stake,
        );

        let mut builder = nimiq_transaction_builder::TransactionBuilder::new();
        builder
            .with_sender(Sender::new_basic(sender.native_ref().clone()))
            .with_recipient(recipient.generate().unwrap())
            .with_value(Coin::ZERO)
            .with_fee(Coin::try_from(fee.unwrap_or(0))?)
            .with_validity_start_height(validity_start_height)
            .with_network_id(to_network_id(network_id)?);

        let proof_builder = builder.generate()?;
        let tx = proof_builder.preliminary_transaction().to_owned();
        Ok(Transaction::from_native(tx))
    }

    /// Unstakes stake from the staking contract and transfers `value` amount of luna (NIM's smallest unit)
    /// from the staker to the recipient.
    ///
    /// The returned transaction is not yet signed. You can sign it e.g. with `tx.sign(keyPair)`.
    ///
    /// Throws when the numbers given for value and fee do not fit within a u64 or the networkId is unknown.
    #[wasm_bindgen(js_name = newUnstake)]
    pub fn new_unstake(
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
        let recipient = Recipient::new_basic(recipient.native_ref().clone());

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
        Ok(Transaction::from_native(tx))
    }

    /// Sets the inactive stake balance of the staker. This is a
    /// signaling transaction and as such does not transfer any value.
    ///
    /// The returned transaction is not yet signed. You can sign it e.g. with `tx.sign(keyPair)`.
    ///
    /// Throws when the number given for fee does not fit within a u64 or the networkId is unknown.
    #[wasm_bindgen(js_name = newSetInactiveStake)]
    pub fn new_set_inactive_stake(
        sender: &Address,
        new_inactive_balance: u64,
        fee: Option<u64>,
        validity_start_height: u32,
        network_id: u8,
    ) -> Result<Transaction, JsError> {
        let mut recipient = Recipient::new_staking_builder();
        recipient.set_inactive_stake(Coin::try_from(new_inactive_balance)?);

        let mut builder = nimiq_transaction_builder::TransactionBuilder::new();
        builder
            .with_sender(Sender::new_basic(sender.native_ref().clone()))
            .with_recipient(recipient.generate().unwrap())
            .with_value(Coin::ZERO)
            .with_fee(Coin::try_from(fee.unwrap_or(0))?)
            .with_validity_start_height(validity_start_height)
            .with_network_id(to_network_id(network_id)?);

        let proof_builder = builder.generate()?;
        let tx = proof_builder.preliminary_transaction().to_owned();
        Ok(Transaction::from_native(tx))
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
        signing_key: PublicKey,
        voting_key_pair: BLSKeyPair,
        fee: Option<u64>,
        validity_start_height: u32,
        network_id: u8,
    ) -> Result<Transaction, JsError> {
        let mut recipient = Recipient::new_staking_builder();
        recipient.create_validator(
            signing_key.native_ref().clone(),
            &voting_key_pair.native_ref().clone(),
            reward_address.native_ref().clone(),
            None,
        );

        let mut builder = nimiq_transaction_builder::TransactionBuilder::new();
        builder
            .with_sender(Sender::new_basic(sender.native_ref().clone()))
            .with_recipient(recipient.generate().unwrap())
            .with_value(Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT))
            .with_fee(Coin::try_from(fee.unwrap_or(0))?)
            .with_validity_start_height(validity_start_height)
            .with_network_id(to_network_id(network_id)?);

        let proof_builder = builder.generate()?;
        let tx = proof_builder.preliminary_transaction().to_owned();
        Ok(Transaction::from_native(tx))
    }

    // pub fn new_update_validator()

    // pub fn new_deactivate_validator()

    // pub fn new_reactivate_validator()

    // pub fn new_delete_validator()
}
