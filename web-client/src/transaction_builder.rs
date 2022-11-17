use wasm_bindgen::prelude::*;

use nimiq_blockchain_interface::AbstractBlockchain;
use nimiq_blockchain_proxy::BlockchainProxy;
use nimiq_primitives::{account::AccountType, coin::Coin, policy::Policy};
use nimiq_transaction_builder::Recipient;

use crate::address::Address;
use crate::transaction::Transaction;
use crate::utils::to_network_id;

/// The TransactionBuilder class provides helper methods to easily create standard types of transactions.
/// It can only be instantiated from a Client with `client.transactionBuilder()`.
#[wasm_bindgen]
pub struct TransactionBuilder {
    network_id: u8,
    blockchain: BlockchainProxy,
}

impl TransactionBuilder {
    pub fn new(network_id: u8, blockchain: BlockchainProxy) -> Self {
        Self {
            network_id,
            blockchain,
        }
    }

    pub fn block_number(&self) -> u32 {
        self.blockchain.read().block_number()
    }
}

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
        &self,
        sender: &Address,
        recipient: &Address,
        value: u64,
        fee: Option<u64>,
        validity_start_height: Option<u32>,
        network_id: Option<u8>,
    ) -> Result<Transaction, JsError> {
        let mut builder = nimiq_transaction_builder::TransactionBuilder::new();
        builder
            .with_sender(sender.native_ref().clone())
            .with_recipient(Recipient::new_basic(recipient.native_ref().clone()))
            .with_value(Coin::try_from(value)?)
            .with_fee(Coin::try_from(fee.unwrap_or(0))?)
            .with_validity_start_height(
                validity_start_height.unwrap_or_else(|| self.block_number()),
            )
            .with_network_id(to_network_id(network_id.unwrap_or(self.network_id))?);

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
        &self,
        sender: &Address,
        recipient: &Address,
        data: Vec<u8>,
        value: u64,
        fee: Option<u64>,
        validity_start_height: Option<u32>,
        network_id: Option<u8>,
    ) -> Result<Transaction, JsError> {
        let mut builder = nimiq_transaction_builder::TransactionBuilder::new();
        builder
            .with_sender(sender.native_ref().clone())
            .with_recipient(Recipient::new_basic_with_data(
                recipient.native_ref().clone(),
                data,
            ))
            .with_value(Coin::try_from(value)?)
            .with_fee(Coin::try_from(fee.unwrap_or(0))?)
            .with_validity_start_height(
                validity_start_height.unwrap_or_else(|| self.block_number()),
            )
            .with_network_id(to_network_id(network_id.unwrap_or(self.network_id))?);

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
        &self,
        sender: &Address,
        delegation: &Address,
        value: u64,
        fee: Option<u64>,
        validity_start_height: Option<u32>,
        network_id: Option<u8>,
    ) -> Result<Transaction, JsError> {
        let mut recipient = Recipient::new_staking_builder();
        recipient.create_staker(Some(delegation.native_ref().clone()));

        let mut builder = nimiq_transaction_builder::TransactionBuilder::new();
        builder
            .with_sender(sender.native_ref().clone())
            .with_recipient(recipient.generate().unwrap())
            .with_value(Coin::try_from(value)?)
            .with_fee(Coin::try_from(fee.unwrap_or(0))?)
            .with_validity_start_height(
                validity_start_height.unwrap_or_else(|| self.block_number()),
            )
            .with_network_id(to_network_id(network_id.unwrap_or(self.network_id))?);

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
        &self,
        sender: &Address,
        staker_address: &Address,
        value: u64,
        fee: Option<u64>,
        validity_start_height: Option<u32>,
        network_id: Option<u8>,
    ) -> Result<Transaction, JsError> {
        let mut recipient = Recipient::new_staking_builder();
        recipient.stake(staker_address.native_ref().clone());

        let mut builder = nimiq_transaction_builder::TransactionBuilder::new();
        builder
            .with_sender(sender.native_ref().clone())
            .with_recipient(recipient.generate().unwrap())
            .with_value(Coin::try_from(value)?)
            .with_fee(Coin::try_from(fee.unwrap_or(0))?)
            .with_validity_start_height(
                validity_start_height.unwrap_or_else(|| self.block_number()),
            )
            .with_network_id(to_network_id(network_id.unwrap_or(self.network_id))?);

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
        &self,
        sender: &Address,
        new_delegation: &Address,
        fee: Option<u64>,
        validity_start_height: Option<u32>,
        network_id: Option<u8>,
    ) -> Result<Transaction, JsError> {
        let mut recipient = Recipient::new_staking_builder();
        recipient.update_staker(Some(new_delegation.native_ref().clone()));

        let mut builder = nimiq_transaction_builder::TransactionBuilder::new();
        builder
            .with_sender(sender.native_ref().clone())
            .with_recipient(recipient.generate().unwrap())
            .with_value(Coin::ZERO)
            .with_fee(Coin::try_from(fee.unwrap_or(0))?)
            .with_validity_start_height(
                validity_start_height.unwrap_or_else(|| self.block_number()),
            )
            .with_network_id(to_network_id(network_id.unwrap_or(self.network_id))?);

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
        &self,
        recipient: &Address,
        value: u64,
        fee: Option<u64>,
        validity_start_height: Option<u32>,
        network_id: Option<u8>,
    ) -> Result<Transaction, JsError> {
        let recipient = Recipient::new_basic(recipient.native_ref().clone());

        let mut builder = nimiq_transaction_builder::TransactionBuilder::new();
        builder
            .with_sender(Policy::STAKING_CONTRACT_ADDRESS)
            .with_sender_type(AccountType::Staking)
            .with_recipient(recipient)
            .with_value(Coin::try_from(value)?)
            .with_fee(Coin::try_from(fee.unwrap_or(0))?)
            .with_validity_start_height(
                validity_start_height.unwrap_or_else(|| self.block_number()),
            )
            .with_network_id(to_network_id(network_id.unwrap_or(self.network_id))?);

        let proof_builder = builder.generate()?;
        let tx = proof_builder.preliminary_transaction().to_owned();
        Ok(Transaction::from_native(tx))
    }

    // pub fn new_create_validator()

    // pub fn new_update_validator()

    // pub fn new_inactivate_validator()

    // pub fn new_reactivate_validator()

    // pub fn new_unpark_validator()

    // pub fn new_delete_validator()
}
