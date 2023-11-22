use nimiq_bls::KeyPair as BlsKeyPair;
use nimiq_hash::Blake2bHash;
use nimiq_keys::{Address, KeyPair, PublicKey};
use nimiq_primitives::{coin::Coin, networks::NetworkId, policy::Policy};
use nimiq_transaction::{
    account::htlc_contract::{AnyHash, PreImage},
    SignatureProof, Transaction,
};
use thiserror::Error;

pub use crate::{proof::TransactionProofBuilder, recipient::Recipient, sender::Sender};

pub mod proof;
pub mod recipient;
pub mod sender;

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
    /// Zero value transactions are called [`signaling transaction`] (also see there for a list of signaling transactions).
    ///
    /// [`signaling transaction`]: struct.TransactionBuilder.html#method.with_value
    #[error("The value must be zero for signaling transactions and cannot be zero for others.")]
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
    sender: Option<Sender>,
    value: Option<Coin>,
    fee: Option<Coin>,
    recipient: Option<Recipient>,
    validity_start_height: Option<u32>,
    network_id: Option<NetworkId>,
}

// Basic builder functionality.
impl TransactionBuilder {
    /// Creates an new, empty `TransactionBuilder`.
    ///
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
    /// use nimiq_transaction_builder::{TransactionBuilder, Recipient, Sender};
    /// use nimiq_keys::Address;
    /// use nimiq_primitives::coin::Coin;
    /// use nimiq_primitives::networks::NetworkId;
    ///
    /// let sender_address = Address::from_any_str("NQ46 MNYU LQ93 GYYS P5DC YA51 L5JP UPUT KR62").unwrap();
    /// let sender = Sender::new_basic(sender_address);
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
    /// assert_eq!(transaction.sender, sender.address());
    /// ```
    pub fn with_required(
        sender: Sender,
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
    ///
    /// The value is a *required* field and must always be set.
    ///
    /// Most transactions have to have non-zero transaction values.
    /// The only exceptions are signaling transactions to:
    /// * `update validator details`
    /// * `retire validators`
    /// * `re-activate validators`
    /// * `update staker details`
    /// * `retire staker funds`
    /// * `re-activate staker funds`
    ///
    /// Signaling transactions have a special status as they also require an additional step
    /// during the proof generation (see [`StakingDataBuilder`]).
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
    ///
    /// The fee is not mandatory and will default to 0 if not provided.
    ///
    /// # Examples
    ///
    /// ```
    /// use nimiq_transaction_builder::{TransactionBuilder, Recipient, Sender};
    /// use nimiq_keys::Address;
    /// use nimiq_primitives::coin::Coin;
    /// use nimiq_primitives::networks::NetworkId;
    ///
    /// let sender = Sender::new_basic(Address::from_any_str("NQ46 MNYU LQ93 GYYS P5DC YA51 L5JP UPUT KR62").unwrap());
    /// let recipient = Recipient::new_basic(
    ///     Address::from_any_str("NQ25 B7NR A1HC V4R2 YRKD 20PR RPGS MNV7 D812").unwrap()
    /// );
    /// let mut builder = TransactionBuilder::with_required(
    ///     sender,
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
    ///
    /// The sender is a *required* field and must always be set.
    ///
    /// # Examples
    ///
    /// ```
    /// use nimiq_transaction_builder::{Sender, TransactionBuilder};
    /// use nimiq_keys::Address;
    /// use nimiq_primitives::coin::Coin;
    ///
    /// let sender = Sender::new_basic(Address::from_any_str("NQ46 MNYU LQ93 GYYS P5DC YA51 L5JP UPUT KR62").unwrap());
    /// let mut builder = TransactionBuilder::new();
    /// builder.with_sender(sender);
    /// ```
    pub fn with_sender(&mut self, sender: Sender) -> &mut Self {
        self.sender = Some(sender);
        self
    }

    /// Sets the transaction's `recipient`.
    ///
    /// [`Recipient`]'s can be easily created using builder methods that will automatically
    /// populate the transaction's data field depending on the chosen type of recipient.
    /// The recipient is a *required* field.
    ///
    /// # Examples
    ///
    /// ```
    /// use nimiq_transaction_builder::{TransactionBuilder, Recipient, Sender};
    /// use nimiq_keys::Address;
    /// use nimiq_primitives::coin::Coin;
    /// use nimiq_primitives::networks::NetworkId;
    ///
    /// let sender = Sender::new_basic(Address::from_any_str("NQ46 MNYU LQ93 GYYS P5DC YA51 L5JP UPUT KR62").unwrap());
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
    ///
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
    ///
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
    ///
    /// In case of a failure, it returns a [`TransactionBuilderError`].
    ///
    /// # Examples
    ///
    /// ```
    /// use nimiq_transaction_builder::{TransactionBuilder, Recipient, Sender};
    /// use nimiq_keys::Address;
    /// use nimiq_primitives::coin::Coin;
    /// use nimiq_primitives::networks::NetworkId;
    ///
    /// let sender = Sender::new_basic(Address::from_any_str("NQ46 MNYU LQ93 GYYS P5DC YA51 L5JP UPUT KR62").unwrap());
    /// let recipient = Recipient::new_basic(Address::from_any_str("NQ25 B7NR A1HC V4R2 YRKD 20PR RPGS MNV7 D812").unwrap());
    ///
    /// let mut builder = TransactionBuilder::with_required(
    ///     sender,
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

        if recipient.is_signaling() != value.is_zero() {
            return Err(TransactionBuilderError::InvalidValue);
        }

        // Currently, the flags for creation & signaling can never occur at the same time.
        let tx = if recipient.is_creation() {
            Transaction::new_contract_creation(
                sender.address(),
                sender.account_type(),
                vec![],
                recipient.account_type(),
                recipient.data(),
                value,
                self.fee.unwrap_or(Coin::ZERO),
                validity_start_height,
                network_id,
            )
        } else if recipient.is_signaling() {
            Transaction::new_signaling(
                sender.address(),
                sender.account_type(),
                recipient.address().unwrap(), // For non-creation recipients, this should never return None.
                recipient.account_type(),
                self.fee.unwrap_or(Coin::ZERO),
                recipient.data(),
                validity_start_height,
                network_id,
            )
        } else {
            Transaction::new_extended(
                sender.address(),
                sender.account_type(),
                sender.data(),
                recipient.address().unwrap(), // For non-creation recipients, this should never return None.
                recipient.account_type(),
                recipient.data(),
                value,
                self.fee.unwrap_or(Coin::ZERO),
                validity_start_height,
                network_id,
            )
        };

        Ok(TransactionProofBuilder::new(tx))
    }
}

// Convenience functionality.
impl TransactionBuilder {
    /// Creates a basic transaction from the address of a given `key_pair` to a basic `recipient`.
    ///
    /// # Arguments
    ///
    ///  - `key_pair`:              The key pair used to sign the outgoing transaction. The
    ///                             transaction value is sent from the basic account belonging to
    ///                             this key pair.
    ///  - `recipient`:             The address of the basic account that will receive the funds.
    ///  - `value`:                 The value that will be sent to the recipient account.
    ///  - `fee`:                   Transaction fee.
    ///  - `validity_start_height`: Block height from which this transaction is valid.
    ///  - `network_id`:            ID of network for which the transaction is meant.
    ///
    /// # Returns
    ///
    /// The finalized transaction.
    ///
    pub fn new_basic(
        key_pair: &KeyPair,
        recipient: Address,
        value: Coin,
        fee: Coin,
        validity_start_height: u32,
        network_id: NetworkId,
    ) -> Result<Transaction, TransactionBuilderError> {
        let mut builder = Self::new();
        builder
            .with_sender(Sender::new_basic(Address::from(key_pair)))
            .with_recipient(Recipient::new_basic(recipient))
            .with_value(value)
            .with_fee(fee)
            .with_validity_start_height(validity_start_height)
            .with_network_id(network_id);

        let proof_builder = builder.generate()?;
        match proof_builder {
            TransactionProofBuilder::Basic(mut builder) => {
                builder.sign_with_key_pair(key_pair);
                Ok(builder.generate().unwrap())
            }
            _ => unreachable!(),
        }
    }

    /// Creates a basic transaction with an arbitrary data field.
    ///
    /// # Arguments
    ///
    ///  - `key_pair`:              The key pair used to sign the outgoing transaction. The
    ///                             transaction value is sent from the basic account belonging to
    ///                             this key pair.
    ///  - `recipient`:             The address of the basic account that will receive the funds.
    ///  - `data`:                  The data that will be stored in the transaction data field.
    ///  - `value`:                 The value that will be sent to the recipient account.
    ///  - `fee`:                   Transaction fee.
    ///  - `validity_start_height`: Block height from which this transaction is valid.
    ///  - `network_id`:            ID of network for which the transaction is meant.
    ///
    /// # Returns
    ///
    /// The finalized transaction.
    ///
    pub fn new_basic_with_data(
        key_pair: &KeyPair,
        recipient: Address,
        data: Vec<u8>,
        value: Coin,
        fee: Coin,
        validity_start_height: u32,
        network_id: NetworkId,
    ) -> Result<Transaction, TransactionBuilderError> {
        let mut builder = Self::new();
        builder
            .with_sender(Sender::new_basic(Address::from(key_pair)))
            .with_recipient(Recipient::new_basic_with_data(recipient, data))
            .with_value(value)
            .with_fee(fee)
            .with_validity_start_height(validity_start_height)
            .with_network_id(network_id);

        let proof_builder = builder.generate()?;
        match proof_builder {
            TransactionProofBuilder::Basic(mut builder) => {
                builder.sign_with_key_pair(key_pair);
                Ok(builder.generate().unwrap())
            }
            _ => unreachable!(),
        }
    }

    /// Creates a transaction that creates a new vesting contract.
    ///
    /// # Arguments
    ///
    ///  - `key_pair`:              The key pair used to sign the outgoing transaction. The vesting
    ///                             contract value is sent from the basic account belonging to this
    ///                             key pair.
    ///  - `owner`:                 The address of the owner of the vesting contract.
    ///  - `start_time`,
    ///    `time_step`,
    ///    `num_steps`:             Create a release schedule of `num_steps` payouts of value
    ///                             starting at `start_time + time_step`.
    ///  - `value`:                 The value for the vesting contract. This is sent from the
    ///                             account belonging to `key_pair`.
    ///  - `fee`:                   Transaction fee.
    ///  - `validity_start_height`: Block height from which this transaction is valid.
    ///  - `network_id`:            ID of network for which the transaction is meant.
    ///
    /// # Returns
    ///
    /// The finalized transaction.
    ///
    pub fn new_create_vesting(
        key_pair: &KeyPair,
        owner: Address,
        start_time: u64,
        time_step: u64,
        num_steps: u32,
        value: Coin,
        fee: Coin,
        validity_start_height: u32,
        network_id: NetworkId,
    ) -> Result<Transaction, TransactionBuilderError> {
        let mut recipient = Recipient::new_vesting_builder(owner);
        recipient.with_steps(value, start_time, time_step, num_steps);

        let mut builder = Self::new();
        builder
            .with_sender(Sender::new_basic(Address::from(key_pair)))
            .with_recipient(recipient.generate().unwrap())
            .with_value(value)
            .with_fee(fee)
            .with_validity_start_height(validity_start_height)
            .with_network_id(network_id);

        let proof_builder = builder.generate()?;
        match proof_builder {
            TransactionProofBuilder::Basic(mut builder) => {
                builder.sign_with_key_pair(key_pair);
                Ok(builder.generate().unwrap())
            }
            _ => unreachable!(),
        }
    }

    /// Creates a transaction that redeems funds from a vesting contract.
    ///
    /// # Arguments
    ///
    ///  - `key_pair`:              The key pair used to sign the transaction. This key pair
    ///                             corresponds to the owner of the vesting contract
    ///  - `contract_address`:      The address of the vesting contract.
    ///  - `recipient`:             The address of the basic account that will receive the funds.
    ///  - `value`:                 The value that will be sent to the recipient account.
    ///  - `fee`:                   Transaction fee.
    ///  - `validity_start_height`: Block height from which this transaction is valid.
    ///  - `network_id`:            ID of network for which the transaction is meant.
    ///
    /// # Returns
    ///
    /// The finalized transaction.
    ///
    pub fn new_redeem_vesting(
        key_pair: &KeyPair,
        contract_address: Address,
        recipient: Address,
        value: Coin,
        fee: Coin,
        validity_start_height: u32,
        network_id: NetworkId,
    ) -> Result<Transaction, TransactionBuilderError> {
        let mut builder = Self::new();
        builder
            .with_sender(Sender::new_vesting(contract_address))
            .with_recipient(Recipient::new_basic(recipient))
            .with_value(value)
            .with_fee(fee)
            .with_validity_start_height(validity_start_height)
            .with_network_id(network_id);

        let proof_builder = builder.generate()?;
        match proof_builder {
            TransactionProofBuilder::Vesting(mut builder) => {
                builder.sign_with_key_pair(key_pair);
                Ok(builder.generate().unwrap())
            }
            _ => unreachable!(),
        }
    }

    /// Creates a transaction that creates a new HTLC contract.
    ///
    /// # Arguments
    ///
    ///  - `key_pair`:              The key pair used to sign the outgoing transaction. The HTLC
    ///                             contract value is sent from the basic account belonging to this
    ///                             key pair.
    ///  - `htlc_sender`:           The address of the sender in the HTLC contract.
    ///  - `htlc_recipient`:        The address of the recipient in the HTLC contract.
    ///  - `hash_root`,
    ///    `hash_count`,
    ///  - `timeout`:               Sets the blockchain height at which the `htlc_sender`
    ///                             automatically gains control over the funds.
    ///  - `value`:                 The value for the vesting contract. This is sent from the
    ///                             account belonging to `key_pair`.
    ///  - `fee`:                   Transaction fee.
    ///  - `validity_start_height`: Block height from which this transaction is valid.
    ///  - `network_id`:            ID of network for which the transaction is meant.
    ///
    /// # Returns
    ///
    /// The finalized transaction.
    ///
    pub fn new_create_htlc(
        key_pair: &KeyPair,
        htlc_sender: Address,
        htlc_recipient: Address,
        hash_root: AnyHash,
        hash_count: u8,
        timeout: u64,
        value: Coin,
        fee: Coin,
        validity_start_height: u32,
        network_id: NetworkId,
    ) -> Result<Transaction, TransactionBuilderError> {
        let mut recipient = Recipient::new_htlc_builder();
        recipient
            .with_sender(htlc_sender)
            .with_recipient(htlc_recipient)
            .with_hash(hash_root, hash_count)
            .with_timeout(timeout);

        let mut builder = Self::new();
        builder
            .with_sender(Sender::new_basic(Address::from(key_pair)))
            .with_recipient(recipient.generate().unwrap())
            .with_value(value)
            .with_fee(fee)
            .with_validity_start_height(validity_start_height)
            .with_network_id(network_id);

        let proof_builder = builder.generate()?;
        match proof_builder {
            TransactionProofBuilder::Basic(mut builder) => {
                builder.sign_with_key_pair(key_pair);
                Ok(builder.generate().unwrap())
            }
            _ => unreachable!(),
        }
    }

    /// Creates a transaction that redeems funds from a HTLC contract using the `RegularTransfer`
    /// method.
    /// The contract stores a `hash_root`. The `htlc_recipient` can withdraw the funds before the
    /// `timeout` has been reached by presenting a hash that will yield the `hash_root`
    /// when re-hashing it `hash_count` times.
    /// By presenting a hash that will yield the `hash_root` after re-hashing it k < `hash_count`
    /// times, the `htlc_recipient` can retrieve 1/k of the funds.
    ///
    /// # Arguments
    ///
    ///  - `key_pair`:              The key pair used to sign the transaction. This key pair
    ///                             corresponds to the `htlc_recipient` in the HTLC contract
    ///  - `contract_address`:      The address of the HTLC contract.
    ///  - `recipient`:             The address of the basic account that will receive the funds.
    ///  - `pre_image`,
    ///    `hash_root`,
    ///    `hash_count`,
    ///  - `value`:                 The value that will be sent to the recipient account.
    ///  - `fee`:                   Transaction fee.
    ///  - `validity_start_height`: Block height from which this transaction is valid.
    ///  - `network_id`:            ID of network for which the transaction is meant.
    ///
    /// # Returns
    ///
    /// The finalized transaction.
    ///
    pub fn new_redeem_htlc_regular(
        key_pair: &KeyPair,
        contract_address: Address,
        recipient: Address,
        pre_image: PreImage,
        hash_root: AnyHash,
        hash_count: u8,
        value: Coin,
        fee: Coin,
        validity_start_height: u32,
        network_id: NetworkId,
    ) -> Result<Transaction, TransactionBuilderError> {
        let mut builder = Self::new();
        builder
            .with_sender(Sender::new_htlc(contract_address))
            .with_recipient(Recipient::new_basic(recipient))
            .with_value(value)
            .with_fee(fee)
            .with_validity_start_height(validity_start_height)
            .with_network_id(network_id);

        let proof_builder = builder.generate()?;
        match proof_builder {
            TransactionProofBuilder::Htlc(mut builder) => {
                let sig = builder.signature_with_key_pair(key_pair);
                builder.regular_transfer(pre_image, hash_count, hash_root, sig);
                Ok(builder.generate().unwrap())
            }
            _ => unreachable!(),
        }
    }

    /// Creates a transaction that redeems funds from a HTLC contract using the `TimeoutResolve`
    /// method. After a blockchain height called `timeout` is reached, the `sender` can withdraw
    /// the funds.
    ///
    /// # Arguments
    ///
    ///  - `key_pair`:              The key pair used to sign the transaction. This key pair
    ///                             corresponds to the `htlc_sender` in the HTLC contract.
    ///  - `contract_address`:      The address of the HTLC contract.
    ///  - `recipient`:             The address of the basic account that will receive the funds.
    ///  - `value`:                 The value that will be sent to the recipient account.
    ///  - `fee`:                   Transaction fee.
    ///  - `validity_start_height`: Block height from which this transaction is valid.
    ///  - `network_id`:            ID of network for which the transaction is meant.
    ///
    /// # Returns
    ///
    /// The finalized transaction.
    ///
    pub fn new_redeem_htlc_timeout(
        key_pair: &KeyPair,
        contract_address: Address,
        recipient: Address,
        value: Coin,
        fee: Coin,
        validity_start_height: u32,
        network_id: NetworkId,
    ) -> Result<Transaction, TransactionBuilderError> {
        let mut builder = Self::new();
        builder
            .with_sender(Sender::new_htlc(contract_address))
            .with_recipient(Recipient::new_basic(recipient))
            .with_value(value)
            .with_fee(fee)
            .with_validity_start_height(validity_start_height)
            .with_network_id(network_id);

        let proof_builder = builder.generate()?;
        match proof_builder {
            TransactionProofBuilder::Htlc(mut builder) => {
                let sig = builder.signature_with_key_pair(key_pair);
                builder.timeout_resolve(sig);
                Ok(builder.generate().unwrap())
            }
            _ => unreachable!(),
        }
    }

    /// Creates a transaction that redeems funds from a HTLC contract using the `EarlyResolve`
    /// method. If both `sender` and `recipient` sign the transaction, the funds can be withdrawn
    /// at any time.
    ///
    /// # Arguments
    ///
    ///  - `contract_address`:         The address of the HTLC contract.
    ///  - `recipient`:                The address of the basic account that will receive the funds.
    ///  - `htlc_sender_signature`:    The signature corresponding to the `htlc_sender` in the HTLC
    ///                                contract.
    ///  - `htlc_recipient_signature`: The signature corresponding to the `htlc_recipient` in the
    ///                                HTLC contract.
    ///  - `value`:                    The value that will be sent to the recipient account.
    ///  - `fee`:                      Transaction fee.
    ///  - `validity_start_height`:    Block height from which this transaction is valid.
    ///  - `network_id`:               ID of network for which the transaction is meant.
    ///
    /// # Returns
    ///
    /// The finalized transaction.
    ///
    pub fn new_redeem_htlc_early(
        contract_address: Address,
        recipient: Address,
        htlc_sender_signature: SignatureProof,
        htlc_recipient_signature: SignatureProof,
        value: Coin,
        fee: Coin,
        validity_start_height: u32,
        network_id: NetworkId,
    ) -> Result<Transaction, TransactionBuilderError> {
        let mut builder = Self::new();
        builder
            .with_sender(Sender::new_htlc(contract_address))
            .with_recipient(Recipient::new_basic(recipient))
            .with_value(value)
            .with_fee(fee)
            .with_validity_start_height(validity_start_height)
            .with_network_id(network_id);

        let proof_builder = builder.generate()?;
        match proof_builder {
            TransactionProofBuilder::Htlc(mut builder) => {
                builder.early_resolve(htlc_sender_signature, htlc_recipient_signature);
                Ok(builder.generate().unwrap())
            }
            _ => unreachable!(),
        }
    }

    /// Creates a signature that can be used to redeem funds from a HTLC contract using the
    /// `EarlyResolve` method. This can be used with both the `htlc_sender` and `htlc_recipient`
    ///  key pairs.
    ///
    /// # Arguments
    ///
    ///  - `key_pair`:              The key pair used to sign the transaction. This key pair
    ///                             corresponds either to the `htlc_sender` or the `htlc_recipient`
    ///                             in the HTLC contract.
    ///  - `contract_address`:      The address of the HTLC contract.
    ///  - `recipient`:             The address of the basic account that will receive the funds.
    ///  - `value`:                 The value that will be sent to the recipient account.
    ///  - `fee`:                   Transaction fee.
    ///  - `validity_start_height`: Block height from which this transaction is valid.
    ///  - `network_id`:            ID of network for which the transaction is meant.
    ///
    /// # Returns
    ///
    /// The signature proof.
    ///
    pub fn sign_htlc_early(
        key_pair: &KeyPair,
        contract_address: Address,
        recipient: Address,
        value: Coin,
        fee: Coin,
        validity_start_height: u32,
        network_id: NetworkId,
    ) -> Result<SignatureProof, TransactionBuilderError> {
        let mut builder = Self::new();
        builder
            .with_sender(Sender::new_htlc(contract_address))
            .with_recipient(Recipient::new_basic(recipient))
            .with_value(value)
            .with_fee(fee)
            .with_validity_start_height(validity_start_height)
            .with_network_id(network_id);

        let proof_builder = builder.generate()?;
        match proof_builder {
            TransactionProofBuilder::Htlc(builder) => Ok(builder.signature_with_key_pair(key_pair)),
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
    /// The finalized transaction.
    ///
    pub fn new_create_staker(
        key_pair: &KeyPair,
        staker_key_pair: &KeyPair,
        delegation: Option<Address>,
        value: Coin,
        fee: Coin,
        validity_start_height: u32,
        network_id: NetworkId,
    ) -> Result<Transaction, TransactionBuilderError> {
        let mut recipient = Recipient::new_staking_builder();
        recipient.create_staker(delegation);

        let mut builder = Self::new();
        builder
            .with_sender(Sender::new_basic(Address::from(key_pair)))
            .with_recipient(recipient.generate().unwrap())
            .with_value(value)
            .with_fee(fee)
            .with_validity_start_height(validity_start_height)
            .with_network_id(network_id);

        let proof_builder = builder.generate()?;
        match proof_builder {
            TransactionProofBuilder::InStaking(mut builder) => {
                builder.sign_with_key_pair(staker_key_pair);
                let mut builder = builder.generate().unwrap().unwrap_basic();
                builder.sign_with_key_pair(key_pair);
                Ok(builder.generate().unwrap())
            }
            _ => unreachable!(),
        }
    }

    /// Creates a staking transaction from the address of a given `key_pair` to a specified
    /// `staker_address`.
    ///
    /// # Arguments
    ///
    ///  - `key_pair`:              The key pair used to sign the outgoing transaction. The
    ///                             stake is sent from the basic account belonging to this key pair.
    ///  - `staker_address`:        The address of the staker that we are sending the stake to.
    ///  - `value`:                 The value of the stake. This is sent from the account
    ///                             belonging to `key_pair`.
    ///  - `fee`:                   Transaction fee.
    ///  - `validity_start_height`: Block height from which this transaction is valid.
    ///  - `network_id`:            ID of network for which the transaction is meant.
    ///
    /// # Returns
    ///
    /// The finalized transaction.
    ///
    pub fn new_stake(
        key_pair: &KeyPair,
        staker_address: Address,
        value: Coin,
        fee: Coin,
        validity_start_height: u32,
        network_id: NetworkId,
    ) -> Result<Transaction, TransactionBuilderError> {
        let mut recipient = Recipient::new_staking_builder();
        recipient.stake(staker_address);

        let mut builder = Self::new();
        builder
            .with_sender(Sender::new_basic(Address::from(key_pair)))
            .with_recipient(recipient.generate().unwrap())
            .with_value(value)
            .with_fee(fee)
            .with_validity_start_height(validity_start_height)
            .with_network_id(network_id);

        let proof_builder = builder.generate()?;
        match proof_builder {
            TransactionProofBuilder::InStaking(mut builder) => {
                builder.sign_with_key_pair(&KeyPair::default());
                let mut builder = builder.generate().unwrap().unwrap_basic();
                builder.sign_with_key_pair(key_pair);
                Ok(builder.generate().unwrap())
            }
            _ => unreachable!(),
        }
    }

    /// Creates an update staker transaction for a given staker that changes the delegation.
    ///
    /// # Arguments
    ///
    ///  - `key_pair`:              The optional key pair used to sign the outgoing transaction. If
    ///                             it is given, the fee will be paid from the basic account
    ///                             belonging to this key pair.
    ///  - `staker_key_pair`:       The key pair used to sign the incoming transaction. The staker
    ///                             address will be derived from this key pair.
    ///  - `delegation`:            The new delegation.
    ///  - `reactivate_all_stake`:  If it should activate all inactive stake to the new delegation.
    ///  - `fee`:                   Transaction fee.
    ///  - `validity_start_height`: Block height from which this transaction is valid.
    ///  - `network_id`:            ID of network for which the transaction is meant.
    ///
    /// # Returns
    ///
    /// The finalized transaction.
    ///
    /// # Note
    ///
    /// This is a *signaling transaction*.
    ///
    pub fn new_update_staker(
        key_pair: Option<&KeyPair>,
        staker_key_pair: &KeyPair,
        new_delegation: Option<Address>,
        reactivate_all_stake: bool,
        fee: Coin,
        validity_start_height: u32,
        network_id: NetworkId,
    ) -> Result<Transaction, TransactionBuilderError> {
        let mut recipient = Recipient::new_staking_builder();
        recipient.update_staker(new_delegation, reactivate_all_stake);

        let mut builder = Self::new();
        builder
            .with_recipient(recipient.generate().unwrap())
            .with_value(Coin::ZERO)
            .with_fee(fee)
            .with_validity_start_height(validity_start_height)
            .with_network_id(network_id);

        match key_pair {
            None => {
                builder.with_sender(Sender::new_basic(Address::from(staker_key_pair)));
            }
            Some(key) => {
                builder.with_sender(Sender::new_basic(Address::from(key)));
            }
        }

        let proof_builder = builder.generate()?;
        let mut staking_data_builder = proof_builder.unwrap_in_staking();
        staking_data_builder.sign_with_key_pair(staker_key_pair);
        let mut builder = staking_data_builder.generate().unwrap().unwrap_basic();
        match key_pair {
            None => builder.sign_with_key_pair(staker_key_pair),
            Some(key) => builder.sign_with_key_pair(key),
        };
        Ok(builder.generate().unwrap())
    }

    /// Creates a set inactive stake transaction for a given staker.
    ///
    /// # Arguments
    ///
    ///  - `key_pair`:              The optional key pair used to sign the outgoing transaction. If
    ///                             it is given, the fee will be paid from the basic account
    ///                             belonging to this key pair.
    ///  - `staker_key_pair`:       The key pair used to sign the incoming transaction. The staker
    ///                             address will be derived from this key pair.
    ///  - `value`:                 The value of the stake to be inactivated. This is moved from the
    ///                             active balance of the staker.
    ///  - `fee`:                   Transaction fee.
    ///  - `validity_start_height`: Block height from which this transaction is valid.
    ///  - `network_id`:            ID of network for which the transaction is meant.
    ///
    /// # Returns
    ///
    /// The finalized transaction.
    ///
    /// # Note
    ///
    /// This is a *signaling transaction*.
    ///
    pub fn new_set_inactive_stake(
        key_pair: Option<&KeyPair>,
        staker_key_pair: &KeyPair,
        value: Coin,
        fee: Coin,
        validity_start_height: u32,
        network_id: NetworkId,
    ) -> Result<Transaction, TransactionBuilderError> {
        let mut recipient = Recipient::new_staking_builder();
        recipient.set_inactive_stake(value);

        let mut builder = Self::new();
        builder
            .with_recipient(recipient.generate().unwrap())
            .with_value(Coin::ZERO)
            .with_fee(fee)
            .with_validity_start_height(validity_start_height)
            .with_network_id(network_id);

        match key_pair {
            None => {
                builder.with_sender(Sender::new_basic(Address::from(staker_key_pair)));
            }
            Some(key) => {
                builder.with_sender(Sender::new_basic(Address::from(key)));
            }
        }

        let proof_builder = builder.generate()?;
        let mut staking_data_builder = proof_builder.unwrap_in_staking();
        staking_data_builder.sign_with_key_pair(staker_key_pair);
        let mut builder = staking_data_builder.generate().unwrap().unwrap_basic();
        match key_pair {
            None => builder.sign_with_key_pair(staker_key_pair),
            Some(key) => builder.sign_with_key_pair(key),
        };
        Ok(builder.generate().unwrap())
    }

    /// Creates a transaction to move stake of a given staker from the staking contract to a
    /// basic `recipient` address.
    ///
    /// # Arguments
    ///
    ///  - `key_pair`:              The key pair used to sign the outgoing transaction. The staker
    ///                             address will be derived from this key pair.
    ///  - `recipient`:             The basic address that will receive the unstaked funds.
    ///  - `value`:                 The value to be moved from the staker.
    ///  - `fee`:                   Transaction fee.
    ///  - `validity_start_height`: Block height from which this transaction is valid.
    ///  - `network_id`:            ID of network for which the transaction is meant.
    ///
    /// # Returns
    ///
    /// The finalized transaction.
    ///
    pub fn new_unstake(
        key_pair: &KeyPair,
        recipient: Address,
        value: Coin,
        fee: Coin,
        validity_start_height: u32,
        network_id: NetworkId,
    ) -> Result<Transaction, TransactionBuilderError> {
        let sender = Sender::new_staking_builder()
            .remove_stake()
            .generate()
            .unwrap();
        let recipient = Recipient::new_basic(recipient);

        let mut builder = Self::new();
        builder
            .with_sender(sender)
            .with_recipient(recipient)
            .with_value(value)
            .with_fee(fee)
            .with_validity_start_height(validity_start_height)
            .with_network_id(network_id);

        let proof_builder = builder.generate()?;
        match proof_builder {
            TransactionProofBuilder::OutStaking(mut builder) => {
                builder.sign_with_key_pair(key_pair);
                Ok(builder.generate().unwrap())
            }
            _ => unreachable!(),
        }
    }

    /// Creates a transaction that creates a new validator.
    ///
    /// # Arguments
    ///
    ///  - `key_pair`:              The key pair used to sign the transaction. The initial stake is
    ///                             sent from the account belonging to this key pair.
    ///  - `cold_key_pair`:         The key pair that will become the validator address. The data is
    ///                             signed using this key pair.
    ///  - `signing_key` :          The Schnorr signing key used by the validator.
    ///  - `voting_key_pair`:       The BLS key pair used by the validator.
    ///  - `reward_address`:        The address to which the staking rewards are sent.
    ///  - `signal_data`:           The signal data showed by the validator.
    ///  - `fee`:                   Transaction fee.
    ///  - `validity_start_height`: Block height from which this transaction is valid.
    ///  - `network_id`:            ID of network for which the transaction is meant.
    ///
    /// # Returns
    ///
    /// The finalized transaction.
    ///
    pub fn new_create_validator(
        key_pair: &KeyPair,
        cold_key_pair: &KeyPair,
        signing_key: PublicKey,
        voting_key_pair: &BlsKeyPair,
        reward_address: Address,
        signal_data: Option<Blake2bHash>,
        fee: Coin,
        validity_start_height: u32,
        network_id: NetworkId,
    ) -> Result<Transaction, TransactionBuilderError> {
        let mut recipient = Recipient::new_staking_builder();
        recipient.create_validator(signing_key, voting_key_pair, reward_address, signal_data);

        let mut builder = Self::new();
        builder
            .with_sender(Sender::new_basic(Address::from(key_pair)))
            .with_recipient(recipient.generate().unwrap())
            .with_value(Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT))
            .with_fee(fee)
            .with_validity_start_height(validity_start_height)
            .with_network_id(network_id);

        let proof_builder = builder.generate()?;
        match proof_builder {
            TransactionProofBuilder::InStaking(mut builder) => {
                builder.sign_with_key_pair(cold_key_pair);
                let mut builder = builder.generate().unwrap().unwrap_basic();
                builder.sign_with_key_pair(key_pair);
                Ok(builder.generate().unwrap())
            }
            _ => unreachable!(),
        }
    }

    /// Creates a transaction that updates the details of a validator.
    ///
    /// # Arguments
    ///
    ///  - `key_pair`:                 The key pair used to sign the transaction. The transaction
    ///                                fee is taken from the account belonging to this key pair.
    ///  - `cold_key_pair`:            The key pair that corresponds to the validator address. The
    ///                                data is signed using this key pair.
    ///  - `new_signing_key`:          The new Schnorr signing key used by the validator.
    ///  - `new_reward_address`:       The new address to which the staking reward is sent.
    ///  - `new_signal_data`:          The new signal data showed by the validator.
    ///  - `new_voting_key_pair`:      The new validator BLS key pair used by the validator.
    ///  - `fee`:                      Transaction fee.
    ///  - `validity_start_height`:    Block height from which this transaction is valid.
    ///  - `network_id`:               ID of network for which the transaction is valid.
    ///
    /// # Returns
    ///
    /// The finalized transaction.
    ///
    /// # Note
    ///
    /// This is a *signaling transaction*.
    ///
    pub fn new_update_validator(
        key_pair: &KeyPair,
        cold_key_pair: &KeyPair,
        new_signing_key: Option<PublicKey>,
        new_voting_key_pair: Option<&BlsKeyPair>,
        new_reward_address: Option<Address>,
        new_signal_data: Option<Option<Blake2bHash>>,
        fee: Coin,
        validity_start_height: u32,
        network_id: NetworkId,
    ) -> Result<Transaction, TransactionBuilderError> {
        let mut recipient = Recipient::new_staking_builder();
        recipient.update_validator(
            new_signing_key,
            new_voting_key_pair,
            new_reward_address,
            new_signal_data,
        );

        let mut builder = Self::new();
        builder
            .with_sender(Sender::new_basic(Address::from(key_pair)))
            .with_recipient(recipient.generate().unwrap())
            .with_value(Coin::ZERO)
            .with_fee(fee)
            .with_validity_start_height(validity_start_height)
            .with_network_id(network_id);

        let proof_builder = builder.generate()?;
        match proof_builder {
            TransactionProofBuilder::InStaking(mut builder) => {
                builder.sign_with_key_pair(cold_key_pair);
                let mut builder = builder.generate().unwrap().unwrap_basic();
                builder.sign_with_key_pair(key_pair);
                Ok(builder.generate().unwrap())
            }
            _ => unreachable!(),
        }
    }

    /// Creates a transaction that deactivates a validator.
    ///
    /// # Arguments
    ///
    ///  - `key_pair`:              The key pair used to sign the transaction. The transaction fee
    ///                             is taken from the account belonging to this key pair.
    ///  - `validator_address`:     The validator address.
    ///  - `signing_key_pair`:      The key pair that corresponds to the validator's signing key.
    ///                             The data is signed using this key pair.
    ///  - `fee`:                   Transaction fee.
    ///  - `validity_start_height`: Block height from which this transaction is valid.
    ///  - `network_id`:            ID of network for which the transaction is valid.
    ///
    /// # Returns
    ///
    /// The finalized transaction.
    ///
    /// # Note
    ///
    /// This is a *signaling transaction*.
    ///
    pub fn new_deactivate_validator(
        key_pair: &KeyPair,
        validator_address: Address,
        signing_key_pair: &KeyPair,
        fee: Coin,
        validity_start_height: u32,
        network_id: NetworkId,
    ) -> Result<Transaction, TransactionBuilderError> {
        let mut recipient = Recipient::new_staking_builder();
        recipient.deactivate_validator(validator_address);

        let mut builder = Self::new();
        builder
            .with_sender(Sender::new_basic(Address::from(key_pair)))
            .with_recipient(recipient.generate().unwrap())
            .with_value(Coin::ZERO)
            .with_fee(fee)
            .with_validity_start_height(validity_start_height)
            .with_network_id(network_id);

        let proof_builder = builder.generate()?;
        match proof_builder {
            TransactionProofBuilder::InStaking(mut builder) => {
                builder.sign_with_key_pair(signing_key_pair);
                let mut builder = builder.generate().unwrap().unwrap_basic();
                builder.sign_with_key_pair(key_pair);
                Ok(builder.generate().unwrap())
            }
            _ => unreachable!(),
        }
    }

    /// Creates a transaction that reactivates an *inactive* validator, i.e. making it *active* again.
    ///
    /// # Arguments
    ///
    ///  - `key_pair`:              The key pair used to sign the transaction. The transaction fee
    ///                             is taken from the account belonging to this key pair.
    ///  - `validator_address`:     The validator address.
    ///  - `signing_key_pair`:      The key pair that corresponds to the validator's signing key.
    ///                             The data is signed using this key pair.
    ///  - `fee`:                   Transaction fee.
    ///  - `validity_start_height`: Block height from which this transaction is valid.
    ///  - `network_id`:            ID of network for which the transaction is valid.
    ///
    /// # Returns
    ///
    /// The finalized transaction.
    ///
    /// # Note
    ///
    /// This is a *signaling transaction*.
    ///
    pub fn new_reactivate_validator(
        key_pair: &KeyPair,
        validator_address: Address,
        signing_key_pair: &KeyPair,
        fee: Coin,
        validity_start_height: u32,
        network_id: NetworkId,
    ) -> Result<Transaction, TransactionBuilderError> {
        let mut recipient = Recipient::new_staking_builder();
        recipient.reactivate_validator(validator_address);

        let mut builder = Self::new();
        builder
            .with_sender(Sender::new_basic(Address::from(key_pair)))
            .with_recipient(recipient.generate().unwrap())
            .with_value(Coin::ZERO)
            .with_fee(fee)
            .with_validity_start_height(validity_start_height)
            .with_network_id(network_id);

        let proof_builder = builder.generate()?;
        match proof_builder {
            TransactionProofBuilder::InStaking(mut builder) => {
                builder.sign_with_key_pair(signing_key_pair);
                let mut builder = builder.generate().unwrap().unwrap_basic();
                builder.sign_with_key_pair(key_pair);
                Ok(builder.generate().unwrap())
            }
            _ => unreachable!(),
        }
    }

    /// Creates a transaction that retires a validator.
    ///
    /// # Arguments
    ///
    ///  - `key_pair`:                 The key pair used to sign the transaction. The transaction
    ///                                fee is taken from the account belonging to this key pair.
    ///  - `cold_key_pair`:            The key pair that corresponds to the validator address. The
    ///                                data is signed using this key pair.
    ///  - `fee`:                      Transaction fee.
    ///  - `validity_start_height`:    Block height from which this transaction is valid.
    ///  - `network_id`:               ID of network for which the transaction is valid.
    ///
    /// # Returns
    ///
    /// The finalized transaction.
    ///
    /// # Note
    ///
    /// This is a *signaling transaction*.
    ///
    pub fn new_retire_validator(
        key_pair: &KeyPair,
        cold_key_pair: &KeyPair,
        fee: Coin,
        validity_start_height: u32,
        network_id: NetworkId,
    ) -> Result<Transaction, TransactionBuilderError> {
        let mut recipient = Recipient::new_staking_builder();
        recipient.retire_validator();

        let mut builder = Self::new();
        builder
            .with_sender(Sender::new_basic(Address::from(key_pair)))
            .with_recipient(recipient.generate().unwrap())
            .with_value(Coin::ZERO)
            .with_fee(fee)
            .with_validity_start_height(validity_start_height)
            .with_network_id(network_id);

        let proof_builder = builder.generate()?;
        match proof_builder {
            TransactionProofBuilder::InStaking(mut builder) => {
                builder.sign_with_key_pair(cold_key_pair);
                let mut builder = builder.generate().unwrap().unwrap_basic();
                builder.sign_with_key_pair(key_pair);
                Ok(builder.generate().unwrap())
            }
            _ => unreachable!(),
        }
    }

    /// Creates a transaction that deletes an *inactive* validator. The validator must have been
    /// *inactive* for the minimum cool-down period.
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
    /// The finalized transaction.
    ///
    pub fn new_delete_validator(
        recipient: Address,
        cold_key_pair: &KeyPair,
        fee: Coin,
        value: Coin,
        validity_start_height: u32,
        network_id: NetworkId,
    ) -> Result<Transaction, TransactionBuilderError> {
        let recipient = Recipient::new_basic(recipient);

        let mut builder = Self::new();
        builder
            .with_sender(
                Sender::new_staking_builder()
                    .delete_validator()
                    .generate()
                    .unwrap(),
            )
            .with_recipient(recipient)
            .with_value(value)
            .with_fee(fee)
            .with_validity_start_height(validity_start_height)
            .with_network_id(network_id);

        let proof_builder = builder.generate()?;
        match proof_builder {
            TransactionProofBuilder::OutStaking(mut builder) => {
                builder.sign_with_key_pair(cold_key_pair);
                Ok(builder.generate().unwrap())
            }
            _ => unreachable!(),
        }
    }
}
