extern crate nimiq_bls as bls;
extern crate nimiq_genesis as genesis;
extern crate nimiq_hash as hash;
extern crate nimiq_keys as keys;
extern crate nimiq_primitives as primitives;
extern crate nimiq_transaction as transaction;
extern crate nimiq_utils as utils;

use thiserror::Error;

use bls::KeyPair as BlsKeyPair;

use keys::{Address, KeyPair};
use primitives::account::AccountType;
use primitives::coin::Coin;
use primitives::networks::NetworkId;
use transaction::Transaction;

pub use crate::proof::TransactionProofBuilder;
pub use crate::recipient::Recipient;
use hash::Blake2bHash;
use primitives::policy::{STAKING_CONTRACT_ADDRESS, VALIDATOR_DEPOSIT};

pub mod proof;
pub mod recipient;

/// Building a transaction can fail if mandatory fields are not set.
/// In these cases, a `TransactionBuilderError` is returned.
#[derive(Debug, Error)]
pub enum TransactionBuilderError {
    /// The `sender` field of the [`TransactionBuilder`] has not been set.
    /// Call [`with_sender`] to set this field.
    ///
    /// [`TransactionBuilder`]: struct.TransactionBuilder.html
    /// [`with_sender`]: struct.TransactionBuilder.html#method.with_sender
    #[error("The transaction sender address is missing.")]
    NoSender,
    /// The `recipient` field of the [`TransactionBuilder`] has not been set.
    /// Call [`with_recipient`] to set this field.
    ///
    /// [`TransactionBuilder`]: struct.TransactionBuilder.html
    /// [`with_recipient`]: struct.TransactionBuilder.html#method.with_recipient
    #[error("The transaction recipient is missing.")]
    NoRecipient,
    /// The `value` field of the [`TransactionBuilder`] has not been set.
    /// Call [`with_value`] to set this field.
    ///
    /// [`TransactionBuilder`]: struct.TransactionBuilder.html
    /// [`with_value`]: struct.TransactionBuilder.html#method.with_value
    #[error("The transaction value is missing.")]
    NoValue,
    /// The `validity_start_height` field of the [`TransactionBuilder`] has not been set.
    /// Call [`with_validity_start_height`] to set this field.
    ///
    /// [`TransactionBuilder`]: struct.TransactionBuilder.html
    /// [`with_validity_start_height`]: struct.TransactionBuilder.html#method.with_validity_start_height
    #[error("The transaction's validity start height is missing.")]
    NoValidityStartHeight,
    /// The `network_id` field of the [`TransactionBuilder`] has not been set.
    /// Call [`with_network_id`] to set this field.
    ///
    /// [`TransactionBuilder`]: struct.TransactionBuilder.html
    /// [`with_network_id`]: struct.TransactionBuilder.html#method.with_network_id
    #[error("The network id is missing.")]
    NoNetworkId,
    /// This error occurs if there are extra restrictions on the sender field induced by the [`Recipient`].
    /// Currently, this is only the case for self transactions on the staking contract that require
    /// the sender to equal the recipient.
    ///
    /// This is only the case when retiring or re-activating stake.
    ///
    /// [`Recipient`]: recipient/enum.Recipient.html
    #[error("The sender is invalid for this recipient.")]
    InvalidSender,
    /// Some transactions require the value to be set to zero (whereas most transactions require a non-zero value).
    /// Zero value transactions are called [`signalling transaction`] (also see there for a list of signalling transactions).
    ///
    /// [`signalling transaction`]: struct.TransactionBuilder.html#method.with_value
    #[error("The value must be zero for signalling transactions and cannot be zero for others.")]
    InvalidValue,
}

/// A helper to build arbitrary transactions.
///
/// The `TransactionBuilder` allows creating transactions for the Nimiq blockchain.
/// It ensures the syntactic validity of data and proof fields through Rust's type system.
///
/// Required fields are:
/// * `sender`
/// * `recipient`
/// * `value`
/// * `validity_start_height`
/// * `network_id`
///
/// After setting all required and relevant fields, the [`generate`] method can be used
/// to create a [`TransactionProofBuilder`].
/// This builder can then be used to create the necessary proof that this transaction may be
/// executed (e.g., through the signature of the sender).
///
/// [`generate`]: struct.TransactionBuilder.html#method.generate
/// [`TransactionProofBuilder`]: proof/enum.TransactionProofBuilder.html
#[derive(Clone, Debug, Default)]
#[cfg_attr(feature = "serde-derive", derive(serde::Serialize, serde::Deserialize))]
pub struct TransactionBuilder {
    sender: Option<Address>,
    sender_type: Option<AccountType>,
    value: Option<Coin>,
    fee: Option<Coin>,
    recipient: Option<Recipient>,
    validity_start_height: Option<u32>,
    network_id: Option<NetworkId>,
}

// Basic builder functionality.
impl TransactionBuilder {
    /// Creates an new, empty `TransactionBuilder`.
    /// Only if all required fields are set, transactions can be generated via [`generate`].
    ///
    /// # Examples
    ///
    /// ```
    /// use nimiq_transaction_builder::TransactionBuilder;
    ///
    /// let builder = TransactionBuilder::new();
    /// assert!(builder.generate().is_err());
    /// ```
    ///
    /// [`generate`]: struct.TransactionBuilder.html#method.generate
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates an new `TransactionBuilder`, setting all required fields.
    ///
    /// # Examples
    ///
    /// ```
    /// use nimiq_transaction_builder::{TransactionBuilder, Recipient};
    /// use nimiq_keys::Address;
    /// use nimiq_primitives::coin::Coin;
    /// use nimiq_primitives::networks::NetworkId;
    ///
    /// let sender = Address::from_any_str("NQ46 MNYU LQ93 GYYS P5DC YA51 L5JP UPUT KR62").unwrap();
    /// let recipient = Recipient::new_basic(
    ///     Address::from_any_str("NQ25 B7NR A1HC V4R2 YRKD 20PR RPGS MNV7 D812").unwrap()
    /// );
    /// let builder = TransactionBuilder::with_required(
    ///     sender.clone(),
    ///     recipient,
    ///     Coin::from_u64_unchecked(100),
    ///     1,
    ///     NetworkId::Main
    /// );
    ///
    /// let proof_builder = builder.generate().unwrap();
    /// let transaction = proof_builder.preliminary_transaction();
    /// assert_eq!(transaction.sender, sender);
    /// ```
    pub fn with_required(
        sender: Address,
        recipient: Recipient,
        value: Coin,
        validity_start_height: u32,
        network_id: NetworkId,
    ) -> Self {
        let mut builder = Self::default();
        builder
            .with_sender(sender)
            .with_recipient(recipient)
            .with_value(value)
            .with_validity_start_height(validity_start_height)
            .with_network_id(network_id);
        builder
    }

    /// Sets the `value` to be transferred by the transaction.
    /// The value is a *required* field and must always be set.
    ///
    /// Most transactions have to have non-zero transaction values.
    /// The only exceptions are signalling transactions to:
    /// * [`update validator details`]
    /// * [`retire validators`]
    /// * [`re-activate validators`]
    /// * [`unpark validators`]
    /// * [`update staker details`]
    /// * [`retire staker funds`]
    /// * [`re-activate staker funds`]
    /// Signalling transactions have a special status as they also require an additional step
    /// during the proof generation (see [`SignallingProofBuilder`]).
    ///
    /// # Examples
    ///
    /// ```
    /// use nimiq_transaction_builder::TransactionBuilder;
    /// use nimiq_primitives::coin::Coin;
    ///
    /// let mut builder = TransactionBuilder::new();
    /// builder.with_value(Coin::from_u64_unchecked(100));
    /// ```
    pub fn with_value(&mut self, value: Coin) -> &mut Self {
        self.value = Some(value);
        self
    }

    /// Sets the transaction `fee` that needs to be paid.
    /// The fee is not mandatory and will default to 0 if not provided.
    ///
    /// # Examples
    ///
    /// ```
    /// use nimiq_transaction_builder::{TransactionBuilder, Recipient};
    /// use nimiq_keys::Address;
    /// use nimiq_primitives::coin::Coin;
    /// use nimiq_primitives::networks::NetworkId;
    ///
    /// let sender = Address::from_any_str("NQ46 MNYU LQ93 GYYS P5DC YA51 L5JP UPUT KR62").unwrap();
    /// let recipient = Recipient::new_basic(
    ///     Address::from_any_str("NQ25 B7NR A1HC V4R2 YRKD 20PR RPGS MNV7 D812").unwrap()
    /// );
    /// let mut builder = TransactionBuilder::with_required(
    ///     sender.clone(),
    ///     recipient,
    ///     Coin::from_u64_unchecked(100),
    ///     1,
    ///     NetworkId::Main
    /// );
    /// builder.with_fee(Coin::from_u64_unchecked(1337));
    ///
    /// let proof_builder = builder.generate().unwrap();
    /// let transaction = proof_builder.preliminary_transaction();
    /// assert_eq!(transaction.fee, Coin::from_u64_unchecked(1337));
    /// ```
    pub fn with_fee(&mut self, fee: Coin) -> &mut Self {
        self.fee = Some(fee);
        self
    }

    /// Sets the `sender` address of a transaction.
    /// The sender is a *required* field and must always be set.
    ///
    /// # Examples
    ///
    /// ```
    /// use nimiq_transaction_builder::TransactionBuilder;
    /// use nimiq_keys::Address;
    /// use nimiq_primitives::coin::Coin;
    ///
    /// let mut builder = TransactionBuilder::new();
    /// builder.with_sender(
    ///     Address::from_any_str("NQ46 MNYU LQ93 GYYS P5DC YA51 L5JP UPUT KR62").unwrap()
    /// );
    /// ```
    pub fn with_sender(&mut self, sender: Address) -> &mut Self {
        self.sender = Some(sender);
        self
    }

    /// Sets the `sender_type`, which describes the account type of the sender.
    /// This field is optional and will default to `AccountType::Basic`.
    ///
    /// Since the sender type determines the type of proof required for the transaction,
    /// it is essential to set it to the correct type.
    ///
    /// The proof builder can be determined as follows:
    /// 1. If the transaction is a [`signalling transaction`], it will be a [`SignallingProofBuilder`].
    /// 2. Otherwise, the following mapping holds depending on `sender_type`:
    ///     - `AccountType::Basic`: [`BasicProofBuilder`]
    ///     - `AccountType::Vesting`: [`BasicProofBuilder`]
    ///     - `AccountType::HTLC`: [`HtlcProofBuilder`]
    ///     - `AccountType::Staking`: [`StakingProofBuilder`]
    ///
    /// # Examples
    ///
    /// ```
    /// use nimiq_transaction_builder::{TransactionBuilder, Recipient};
    /// use nimiq_keys::Address;
    /// use nimiq_primitives::coin::Coin;
    /// use nimiq_primitives::networks::NetworkId;
    /// use nimiq_primitives::account::AccountType;
    ///
    /// let sender = Address::from_any_str("NQ46 MNYU LQ93 GYYS P5DC YA51 L5JP UPUT KR62").unwrap();
    /// let recipient = Recipient::new_basic(
    ///     Address::from_any_str("NQ25 B7NR A1HC V4R2 YRKD 20PR RPGS MNV7 D812").unwrap()
    /// );
    /// let mut builder = TransactionBuilder::with_required(
    ///     sender.clone(),
    ///     recipient,
    ///     Coin::from_u64_unchecked(100),
    ///     1,
    ///     NetworkId::Main
    /// );
    /// builder.with_sender_type(AccountType::HTLC);
    ///
    /// let proof_builder = builder.generate().unwrap();
    /// let transaction = proof_builder.preliminary_transaction();
    /// assert_eq!(transaction.sender_type, AccountType::HTLC);
    ///
    /// // A HTLC sender type will result in a HtlcProofBuilder.
    /// let htlc_proof_builder = proof_builder.unwrap_htlc();
    /// ```
    ///
    /// [`signalling transaction`]: struct.TransactionBuilder.html#method.with_value
    /// [`SignallingProofBuilder`]: proof/staking_contract/struct.SignallingProofBuilder.html
    /// [`BasicProofBuilder`]: proof/struct.BasicProofBuilder.html
    /// [`HtlcProofBuilder`]: proof/htlc_contract/struct.HtlcProofBuilder.html
    /// [`StakingProofBuilder`]: proof/staking_contract/struct.StakingRecipientBuilder.html
    pub fn with_sender_type(&mut self, sender_type: AccountType) -> &mut Self {
        self.sender_type = Some(sender_type);
        self
    }

    /// Sets the transaction's `recipient`.
    /// [`Recipient`]'s can be easily created using builder methods that will automatically
    /// populate the transaction's data field depending on the chosen type of recipient.
    /// The recipient is a *required* field.
    ///
    /// # Examples
    ///
    /// ```
    /// use nimiq_transaction_builder::{TransactionBuilder, Recipient};
    /// use nimiq_keys::Address;
    /// use nimiq_primitives::coin::Coin;
    /// use nimiq_primitives::networks::NetworkId;
    ///
    /// let sender = Address::from_any_str("NQ46 MNYU LQ93 GYYS P5DC YA51 L5JP UPUT KR62").unwrap();
    /// let recipient = Recipient::new_basic(
    ///     Address::from_any_str("NQ25 B7NR A1HC V4R2 YRKD 20PR RPGS MNV7 D812").unwrap()
    /// );
    /// let mut builder = TransactionBuilder::with_required(
    ///     sender.clone(),
    ///     recipient,
    ///     Coin::from_u64_unchecked(100),
    ///     1,
    ///     NetworkId::Main
    /// );
    /// builder.with_fee(Coin::from_u64_unchecked(1337));
    ///
    /// let proof_builder = builder.generate().unwrap();
    /// let transaction = proof_builder.preliminary_transaction();
    /// assert_eq!(
    ///     transaction.recipient,
    ///     Address::from_any_str("NQ25 B7NR A1HC V4R2 YRKD 20PR RPGS MNV7 D812").unwrap()
    /// );
    /// ```
    ///
    /// [`Recipient`]: recipient/enum.Recipient.html
    pub fn with_recipient(&mut self, recipient: Recipient) -> &mut Self {
        self.recipient = Some(recipient);
        self
    }

    /// Sets the `network_id` for the transaction.
    /// The network id is a *required* field and must always be set.
    /// It restricts the validity of the transaction to a network and prevents replay attacks.
    ///
    /// # Examples
    ///
    /// ```
    /// use nimiq_transaction_builder::TransactionBuilder;
    /// use nimiq_primitives::networks::NetworkId;
    ///
    /// let mut builder = TransactionBuilder::new();
    /// builder.with_network_id(NetworkId::Main);
    /// ```
    pub fn with_network_id(&mut self, network_id: NetworkId) -> &mut Self {
        self.network_id = Some(network_id);
        self
    }

    /// Sets the `validity_start_height` for the transaction.
    /// The validity start height is a *required* field and must always be set.
    /// It restricts the validity of the transaction to a blockchain height
    /// and prevents replay attacks.
    ///
    /// In most cases, this number should be set to the current blockchain height.
    ///
    /// # Examples
    ///
    /// ```
    /// use nimiq_transaction_builder::TransactionBuilder;
    ///
    /// let mut builder = TransactionBuilder::new();
    /// builder.with_validity_start_height(13377);
    /// ```
    pub fn with_validity_start_height(&mut self, validity_start_height: u32) -> &mut Self {
        self.validity_start_height = Some(validity_start_height);
        self
    }

    /// This method tries putting together the preliminary transaction
    /// in order to move to the proof building phase by returning a [`TransactionProofBuilder`].
    /// In case of a failure, it returns a [`TransactionBuilderError`].
    ///
    /// # Examples
    ///
    /// ```
    /// use nimiq_transaction_builder::{TransactionBuilder, Recipient};
    /// use nimiq_keys::Address;
    /// use nimiq_primitives::coin::Coin;
    /// use nimiq_primitives::networks::NetworkId;
    ///
    /// let sender = Address::from_any_str("NQ46 MNYU LQ93 GYYS P5DC YA51 L5JP UPUT KR62").unwrap();
    /// let recipient = Recipient::new_basic(
    ///     Address::from_any_str("NQ25 B7NR A1HC V4R2 YRKD 20PR RPGS MNV7 D812").unwrap()
    /// );
    /// let mut builder = TransactionBuilder::with_required(
    ///     sender.clone(),
    ///     recipient,
    ///     Coin::from_u64_unchecked(100),
    ///     1,
    ///     NetworkId::Main
    /// );
    /// builder.with_fee(Coin::from_u64_unchecked(1337));
    ///
    /// let proof_builder = builder.generate().unwrap();
    /// let transaction = proof_builder.preliminary_transaction();
    /// ```
    ///
    /// [`TransactionProofBuilder`]: proof/enum.TransactionProofBuilder.html
    /// [`TransactionBuilderError`]: enum.TransactionBuilderError.html
    pub fn generate(self) -> Result<TransactionProofBuilder, TransactionBuilderError> {
        let sender = self.sender.ok_or(TransactionBuilderError::NoSender)?;
        let recipient = self.recipient.ok_or(TransactionBuilderError::NoRecipient)?;

        let value = self.value.ok_or(TransactionBuilderError::NoValue)?;
        let validity_start_height = self
            .validity_start_height
            .ok_or(TransactionBuilderError::NoValidityStartHeight)?;
        let network_id = self
            .network_id
            .ok_or(TransactionBuilderError::NoNetworkId)?;

        if recipient.is_signalling() != value.is_zero() {
            return Err(TransactionBuilderError::InvalidValue);
        }

        // Currently, the flags for creation & signalling can never occur at the same time.
        let tx = if recipient.is_creation() {
            Transaction::new_contract_creation(
                recipient.data(),
                sender,
                self.sender_type.unwrap_or(AccountType::Basic),
                recipient.account_type(),
                value,
                self.fee.unwrap_or(Coin::ZERO),
                validity_start_height,
                network_id,
            )
        } else if recipient.is_signalling() {
            Transaction::new_signalling(
                sender,
                self.sender_type.unwrap_or(AccountType::Basic),
                recipient.address().unwrap(), // For non-creation recipients, this should never return None.
                recipient.account_type(),
                value,
                self.fee.unwrap_or(Coin::ZERO),
                recipient.data(),
                validity_start_height,
                network_id,
            )
        } else {
            Transaction::new_extended(
                sender,
                self.sender_type.unwrap_or(AccountType::Basic),
                recipient.address().unwrap(), // For non-creation recipients, this should never return None.
                recipient.account_type(),
                value,
                self.fee.unwrap_or(Coin::ZERO),
                recipient.data(),
                validity_start_height,
                network_id,
            )
        };

        Ok(TransactionProofBuilder::new(tx))
    }
}

// Convenience functionality.
impl TransactionBuilder {
    /// Creates a simple transaction from the address of a given `key_pair` to a basic `recipient`.
    pub fn new_simple(
        key_pair: &KeyPair,
        recipient: Address,
        value: Coin,
        fee: Coin,
        validity_start_height: u32,
        network_id: NetworkId,
    ) -> Transaction {
        let sender = Address::from(key_pair);
        let mut builder = Self::new();
        builder
            .with_sender(sender)
            .with_recipient(Recipient::new_basic(recipient))
            .with_value(value)
            .with_fee(fee)
            .with_validity_start_height(validity_start_height)
            .with_network_id(network_id);

        let proof_builder = builder.generate().unwrap();
        match proof_builder {
            TransactionProofBuilder::Basic(mut builder) => {
                builder.sign_with_key_pair(key_pair);
                builder.generate().unwrap()
            }
            _ => unreachable!(),
        }
    }

    /// Creates a transaction that creates a new staker with a given initial stake and delegation.
    ///
    /// # Arguments
    ///
    ///  - `key_pair`:              The key pair used to sign the outgoing transaction. The initial
    ///                             stake is sent from the basic account belonging to this key pair.
    ///  - `staker_key_pair`:       The key pair used to sign the incoming transaction. The staker
    ///                             address will be derived from this key pair.
    ///  - `delegation`:            The (optional) delegation to a validator.
    ///  - `value`:                 The value for the initial stake. This is sent from the account
    ///                             belonging to `key_pair`.
    ///  - `fee`:                   Transaction fee.
    ///  - `validity_start_height`: Block height from which this transaction is valid.
    ///  - `network_id`:            ID of network for which the transaction is meant.
    ///
    /// # Returns
    ///
    /// The finalized transaction (signed using `key_pair`).
    ///
    pub fn new_create_staker(
        key_pair: &KeyPair,
        staker_key_pair: &KeyPair,
        delegation: Option<Address>,
        value: Coin,
        fee: Coin,
        validity_start_height: u32,
        network_id: NetworkId,
    ) -> Transaction {
        let mut recipient = Recipient::new_staking_builder();
        recipient.create_staker(delegation);

        let mut builder = Self::new();
        builder
            .with_sender(Address::from(key_pair))
            .with_recipient(recipient.generate().unwrap())
            .with_value(value)
            .with_fee(fee)
            .with_validity_start_height(validity_start_height)
            .with_network_id(network_id);

        let proof_builder = builder.generate().unwrap();
        match proof_builder {
            TransactionProofBuilder::InStaking(mut builder) => {
                builder.sign_with_key_pair(staker_key_pair);
                let mut builder = builder.generate().unwrap().unwrap_basic();
                builder.sign_with_key_pair(key_pair);
                builder.generate().unwrap()
            }
            _ => unreachable!(),
        }
    }

    /// Creates a staking transaction from the address of a given `key_pair` to a specified `staker_address`.
    ///
    /// # TODO
    ///
    ///  - Allow to add funds to the staker address.
    ///
    pub fn new_stake(
        key_pair: &KeyPair,
        staker_address: Address,
        value: Coin,
        fee: Coin,
        validity_start_height: u32,
        network_id: NetworkId,
    ) -> Transaction {
        let mut recipient = Recipient::new_staking_builder();
        recipient.stake(staker_address);

        let mut builder = Self::new();
        builder
            .with_sender(Address::from(key_pair))
            .with_recipient(recipient.generate().unwrap())
            .with_value(value)
            .with_fee(fee)
            .with_validity_start_height(validity_start_height)
            .with_network_id(network_id);

        let proof_builder = builder.generate().unwrap();
        match proof_builder {
            TransactionProofBuilder::InStaking(mut builder) => {
                builder.sign_with_key_pair(key_pair);
                let mut builder = builder.generate().unwrap().unwrap_basic();
                builder.sign_with_key_pair(key_pair);
                builder.generate().unwrap()
            }
            _ => unreachable!(),
        }
    }

    /// Creates a update staker transaction for a given staker that changes the delegation. It pays
    /// fees from the staker's active or inactive balance.
    pub fn new_update_staker(
        staker_key_pair: &KeyPair,
        new_delegation: Option<Address>,
        from_active_balance: bool,
        fee: Coin,
        validity_start_height: u32,
        network_id: NetworkId,
    ) -> Transaction {
        let staking_contract_address = Address::from_any_str(STAKING_CONTRACT_ADDRESS).unwrap();
        let mut recipient = Recipient::new_staking_builder();
        recipient.update_staker(new_delegation);

        let mut builder = Self::new();
        builder
            .with_sender(staking_contract_address)
            .with_sender_type(AccountType::Staking)
            .with_recipient(recipient.generate().unwrap())
            .with_value(Coin::ZERO)
            .with_fee(fee)
            .with_validity_start_height(validity_start_height)
            .with_network_id(network_id);

        let proof_builder = builder.generate().unwrap();
        match proof_builder {
            TransactionProofBuilder::InStaking(mut builder) => {
                builder.sign_with_key_pair(staker_key_pair);
                let mut builder = builder.generate().unwrap().unwrap_out_staking();
                builder.deduct_fees(from_active_balance, staker_key_pair);
                builder.generate().unwrap()
            }
            _ => unreachable!(),
        }
    }

    /// Retires the stake of a given staker.  It pays
    /// fees from the staker's active or inactive balance.
    pub fn new_retire_staker(
        staker_key_pair: &KeyPair,
        from_active_balance: bool,
        value: Coin,
        fee: Coin,
        validity_start_height: u32,
        network_id: NetworkId,
    ) -> Transaction {
        let staking_contract_address = Address::from_any_str(STAKING_CONTRACT_ADDRESS).unwrap();
        let mut recipient = Recipient::new_staking_builder();
        recipient.retire_stake(value);

        let mut builder = Self::new();
        builder
            .with_sender(staking_contract_address)
            .with_sender_type(AccountType::Staking)
            .with_recipient(recipient.generate().unwrap())
            .with_value(Coin::ZERO)
            .with_fee(fee)
            .with_validity_start_height(validity_start_height)
            .with_network_id(network_id);

        let proof_builder = builder.generate().unwrap();
        match proof_builder {
            TransactionProofBuilder::InStaking(mut builder) => {
                builder.sign_with_key_pair(staker_key_pair);
                let mut builder = builder.generate().unwrap().unwrap_out_staking();
                builder.deduct_fees(from_active_balance, staker_key_pair);
                builder.generate().unwrap()
            }
            _ => unreachable!(),
        }
    }

    /// Re-activates the stake from a given staker. It pays
    /// fees from the staker's active or inactive balance.
    pub fn new_reactivate_staker(
        staker_key_pair: &KeyPair,
        from_active_balance: bool,
        value: Coin,
        fee: Coin,
        validity_start_height: u32,
        network_id: NetworkId,
    ) -> Transaction {
        let staking_contract_address = Address::from_any_str(STAKING_CONTRACT_ADDRESS).unwrap();
        let mut recipient = Recipient::new_staking_builder();
        recipient.reactivate_stake(value);

        let mut builder = Self::new();
        builder
            .with_sender(staking_contract_address)
            .with_sender_type(AccountType::Staking)
            .with_recipient(recipient.generate().unwrap())
            .with_value(Coin::ZERO)
            .with_fee(fee)
            .with_validity_start_height(validity_start_height)
            .with_network_id(network_id);

        let proof_builder = builder.generate().unwrap();
        match proof_builder {
            TransactionProofBuilder::InStaking(mut builder) => {
                builder.sign_with_key_pair(staker_key_pair);
                let mut builder = builder.generate().unwrap().unwrap_out_staking();
                builder.deduct_fees(from_active_balance, staker_key_pair);
                builder.generate().unwrap()
            }
            _ => unreachable!(),
        }
    }

    /// Creates a transaction to move inactive/retired stake of a given staker
    /// from the staking contract to a new basic `recipient` address.
    ///
    /// Note that unstaking transactions can only be executed after the cooldown period has passed.
    pub fn new_unstake(
        key_pair: &KeyPair,
        recipient: Address,
        value: Coin,
        fee: Coin,
        validity_start_height: u32,
        network_id: NetworkId,
    ) -> Transaction {
        let staking_contract_address = Address::from_any_str(STAKING_CONTRACT_ADDRESS).unwrap();
        let recipient = Recipient::new_basic(recipient);

        let mut builder = Self::new();
        builder
            .with_sender(staking_contract_address)
            .with_sender_type(AccountType::Staking)
            .with_recipient(recipient)
            .with_value(value)
            .with_fee(fee)
            .with_validity_start_height(validity_start_height)
            .with_network_id(network_id);

        let proof_builder = builder.generate().unwrap();
        match proof_builder {
            TransactionProofBuilder::OutStaking(mut builder) => {
                builder.unstake(key_pair);
                builder.generate().unwrap()
            }
            _ => unreachable!(),
        }
    }

    /// Creates a transaction that creates a new validator.
    ///
    /// # Arguments
    ///
    ///  - `key_pair`:              The key pair used to sign the transaction. The initial stake is sent from the
    ///                             account belonging to this key pair.
    ///  - `cold_key_pair`:         The key pair that will become the validator address. The data is
    ///                             signed using this key pair.
    ///  - `warm_address`:          The address corresponding to the warm key used by the validator.
    ///  - `validator_key_pair`:    The BLS key pair used by the validator.
    ///  - `reward_address`:        The address to which the staking rewards are sent.
    ///  - `signal_data`:           The signal data showed by the validator.
    ///  - `fee`:                   Transaction fee.
    ///  - `validity_start_height`: Block height from which this transaction is valid.
    ///  - `network_id`:            ID of network for which the transaction is meant.
    ///
    /// # Returns
    ///
    /// The finalized transaction (signed using `key_pair`).
    ///
    pub fn new_create_validator(
        key_pair: &KeyPair,
        cold_key_pair: &KeyPair,
        warm_address: Address,
        validator_key_pair: &BlsKeyPair,
        reward_address: Address,
        signal_data: Option<Blake2bHash>,
        fee: Coin,
        validity_start_height: u32,
        network_id: NetworkId,
    ) -> Transaction {
        let mut recipient = Recipient::new_staking_builder();
        recipient.create_validator(
            warm_address,
            validator_key_pair,
            reward_address,
            signal_data,
        );

        let mut builder = Self::new();
        builder
            .with_sender(Address::from(key_pair))
            .with_recipient(recipient.generate().unwrap())
            .with_value(Coin::from_u64_unchecked(VALIDATOR_DEPOSIT))
            .with_fee(fee)
            .with_validity_start_height(validity_start_height)
            .with_network_id(network_id);

        let proof_builder = builder.generate().unwrap();
        match proof_builder {
            TransactionProofBuilder::InStaking(mut builder) => {
                builder.sign_with_key_pair(cold_key_pair);
                let mut builder = builder.generate().unwrap().unwrap_basic();
                builder.sign_with_key_pair(key_pair);
                builder.generate().unwrap()
            }
            _ => unreachable!(),
        }
    }

    /// Creates a transaction that updates the details of a validator.
    ///
    /// # Arguments
    ///
    ///  - `key_pair`:                 The key pair used to sign the transaction. The transaction fee is taken from the
    ///                                account belonging to this key pair.
    ///  - `cold_key_pair`:            The key pair that corresponds to the validator address. The data is
    ///                                signed using this key pair.
    ///  - `new_warm_address`:         The address corresponding to the new warm key used by the validator.
    ///  - `new_reward_address`:       The new address to which the staking reward is sent.
    ///  - `new_signal_data`:          The new signal data showed by the validator.
    ///  - `new_validator_key_pair`:   The new validator BLS key pair used by the validator.
    ///  - `fee`:                      Transaction fee.
    ///  - `validity_start_height`:    Block height from which this transaction is valid.
    ///  - `network_id`:               ID of network for which the transaction is valid.
    ///
    /// # Returns
    ///
    /// The finalized transaction (signed using `key_pair`).
    ///
    /// # Note
    ///
    /// This is a *signalling transaction*.
    ///
    pub fn new_update_validator(
        key_pair: &KeyPair,
        cold_key_pair: &KeyPair,
        new_warm_address: Option<Address>,
        new_validator_key_pair: Option<&BlsKeyPair>,
        new_reward_address: Option<Address>,
        new_signal_data: Option<Option<Blake2bHash>>,
        fee: Coin,
        validity_start_height: u32,
        network_id: NetworkId,
    ) -> Transaction {
        let mut recipient = Recipient::new_staking_builder();
        recipient.update_validator(
            new_warm_address,
            new_validator_key_pair,
            new_reward_address,
            new_signal_data,
        );

        let mut builder = Self::new();
        builder
            .with_sender(Address::from(key_pair))
            .with_recipient(recipient.generate().unwrap())
            .with_value(Coin::ZERO)
            .with_fee(fee)
            .with_validity_start_height(validity_start_height)
            .with_network_id(network_id);

        let proof_builder = builder.generate().unwrap();
        match proof_builder {
            TransactionProofBuilder::InStaking(mut builder) => {
                builder.sign_with_key_pair(cold_key_pair);
                let mut builder = builder.generate().unwrap().unwrap_basic();
                builder.sign_with_key_pair(key_pair);
                builder.generate().unwrap()
            }
            _ => unreachable!(),
        }
    }

    /// Creates a transaction that retires a validator, i.e. making it *inactive*.
    ///
    /// # Arguments
    ///
    ///  - `key_pair`:              The key pair used to sign the transaction. The transaction fee is taken from the
    ///                             account belonging to this key pair.
    ///  - `validator_address`:     The validator address.
    ///  - `warm_key_pair`:         The key pair that corresponds to the validator's warm address.
    ///                             The data is signed using this key pair.
    ///  - `fee`:                   Transaction fee.
    ///  - `validity_start_height`: Block height from which this transaction is valid.
    ///  - `network_id`:            ID of network for which the transaction is valid.
    ///
    /// # Returns
    ///
    /// The finalized transaction (signed using `key_pair`).
    ///
    /// # Note
    ///
    /// This is a *signalling transaction*.
    ///
    pub fn new_retire_validator(
        key_pair: &KeyPair,
        validator_address: Address,
        warm_key_pair: &KeyPair,
        fee: Coin,
        validity_start_height: u32,
        network_id: NetworkId,
    ) -> Transaction {
        let mut recipient = Recipient::new_staking_builder();
        recipient.retire_validator(validator_address);

        let mut builder = Self::new();
        builder
            .with_sender(Address::from(key_pair))
            .with_recipient(recipient.generate().unwrap())
            .with_value(Coin::ZERO)
            .with_fee(fee)
            .with_validity_start_height(validity_start_height)
            .with_network_id(network_id);

        let proof_builder = builder.generate().unwrap();
        match proof_builder {
            TransactionProofBuilder::InStaking(mut builder) => {
                builder.sign_with_key_pair(warm_key_pair);
                let mut builder = builder.generate().unwrap().unwrap_basic();
                builder.sign_with_key_pair(key_pair);
                builder.generate().unwrap()
            }
            _ => unreachable!(),
        }
    }

    /// Creates a transaction that reactivates an *inactive* validator, i.e. making it *active* again.
    ///
    /// # Arguments
    ///
    ///  - `key_pair`:              The key pair used to sign the transaction. The transaction fee is taken from the
    ///                             account belonging to this key pair.
    ///  - `validator_address`:     The validator address.
    ///  - `warm_key_pair`:         The key pair that corresponds to the validator's warm address.
    ///                             The data is signed using this key pair.
    ///  - `fee`:                   Transaction fee.
    ///  - `validity_start_height`: Block height from which this transaction is valid.
    ///  - `network_id`:            ID of network for which the transaction is valid.
    ///
    /// # Returns
    ///
    /// The finalized transaction (signed using `key_pair`).
    ///
    /// # Note
    ///
    /// This is a *signalling transaction*.
    ///
    pub fn new_reactivate_validator(
        key_pair: &KeyPair,
        validator_address: Address,
        warm_key_pair: &KeyPair,
        fee: Coin,
        validity_start_height: u32,
        network_id: NetworkId,
    ) -> Transaction {
        let mut recipient = Recipient::new_staking_builder();
        recipient.reactivate_validator(validator_address);

        let mut builder = Self::new();
        builder
            .with_sender(Address::from(key_pair))
            .with_recipient(recipient.generate().unwrap())
            .with_value(Coin::ZERO)
            .with_fee(fee)
            .with_validity_start_height(validity_start_height)
            .with_network_id(network_id);

        let proof_builder = builder.generate().unwrap();
        match proof_builder {
            TransactionProofBuilder::InStaking(mut builder) => {
                builder.sign_with_key_pair(warm_key_pair);
                let mut builder = builder.generate().unwrap().unwrap_basic();
                builder.sign_with_key_pair(key_pair);
                builder.generate().unwrap()
            }
            _ => unreachable!(),
        }
    }

    /// Creates a transaction that unparks a *parked* validator, i.e. making it *active* again.
    ///
    /// # Arguments
    ///
    ///  - `key_pair`:              The key pair used to sign the transaction. The transaction fee is taken from the
    ///                             account belonging to this key pair.
    ///  - `validator_address`:     The validator address.
    ///  - `warm_key_pair`:         The key pair that corresponds to the validator's warm address.
    ///                             The data is signed using this key pair.
    ///  - `fee`:                   Transaction fee.
    ///  - `validity_start_height`: Block height from which this transaction is valid.
    ///  - `network_id`:            ID of network for which the transaction is valid.
    ///
    /// # Returns
    ///
    /// The finalized transaction (signed using `key_pair`).
    ///
    /// # Note
    ///
    /// This is a *signalling transaction*.
    ///
    pub fn new_unpark_validator(
        key_pair: &KeyPair,
        validator_address: Address,
        warm_key_pair: &KeyPair,
        fee: Coin,
        validity_start_height: u32,
        network_id: NetworkId,
    ) -> Transaction {
        let mut recipient = Recipient::new_staking_builder();
        recipient.unpark_validator(validator_address);

        let mut builder = Self::new();
        builder
            .with_sender(Address::from(key_pair))
            .with_recipient(recipient.generate().unwrap())
            .with_value(Coin::ZERO)
            .with_fee(fee)
            .with_validity_start_height(validity_start_height)
            .with_network_id(network_id);

        let proof_builder = builder.generate().unwrap();
        match proof_builder {
            TransactionProofBuilder::InStaking(mut builder) => {
                builder.sign_with_key_pair(warm_key_pair);
                let mut builder = builder.generate().unwrap().unwrap_basic();
                builder.sign_with_key_pair(key_pair);
                builder.generate().unwrap()
            }
            _ => unreachable!(),
        }
    }

    /// Creates a transaction that drops an *inactive* validator. The validator must have been *inactive* for the
    /// minimum cool-down period.
    ///
    /// # Arguments
    ///
    ///  - `recipient`:             The recipient of the staked funds.
    ///  - `cold_key_pair`:         The key pair that corresponds to the validator address. The
    ///                             transaction is signed using this key pair.
    ///  - `fee`:                   Transaction fee. The fee is subtracted from the staked funds.
    ///  - `validity_start_height`: Block height from which this transaction is valid.
    ///  - `network_id`:            ID of network for which the transaction is valid.
    ///
    /// # Returns
    ///
    /// The finalized transaction (signed using `cold_key_pair`).
    ///
    pub fn new_drop_validator(
        recipient: Address,
        cold_key_pair: &KeyPair,
        fee: Coin,
        validity_start_height: u32,
        network_id: NetworkId,
    ) -> Transaction {
        let staking_contract_address = Address::from_any_str(STAKING_CONTRACT_ADDRESS).unwrap();
        let recipient = Recipient::new_basic(recipient);

        let mut builder = Self::new();
        builder
            .with_sender(staking_contract_address)
            .with_sender_type(AccountType::Staking)
            .with_recipient(recipient)
            .with_value(Coin::from_u64_unchecked(VALIDATOR_DEPOSIT) - fee)
            .with_fee(fee)
            .with_validity_start_height(validity_start_height)
            .with_network_id(network_id);

        let proof_builder = builder.generate().unwrap();
        match proof_builder {
            TransactionProofBuilder::OutStaking(mut builder) => {
                builder.unstake(cold_key_pair);
                builder.generate().unwrap()
            }
            _ => unreachable!(),
        }
    }
}
