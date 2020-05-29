use beserial::Serialize;
use keys::Address;
use nimiq_account::AccountType;
use transaction::account::htlc_contract::CreationTransactionData as HtlcCreationData;
use transaction::account::vesting_contract::CreationTransactionData as VestingCreationData;

use crate::recipient::htlc_contract::HtlcRecipientBuilder;
use crate::recipient::staking_contract::{StakingRecipientBuilder, StakingTransaction};
use crate::recipient::vesting_contract::VestingRecipientBuilder;

pub mod htlc_contract;
pub mod staking_contract;
pub mod vesting_contract;

/// A `Recipient` describes the recipient of a transaction.
/// This also determines the data field of the transaction to be built.
///
/// New contracts can be created using dedicated builders as described below.
///
/// There are four types of recipients:
/// - basic recipients that can be built with [`new_basic`]
/// - HTLC contracts that can be set up with a builder using [`new_htlc_builder`]
/// - vesting contracts that can be set up with a builder using [`new_vesting_builder`]
/// - actions on the staking contract that built with [`new_staking_builder`]
///
/// [`new_basic`]: enum.Recipient.html#method.new_basic
/// [`new_htlc_builder`]: enum.Recipient.html#method.new_htlc_builder
/// [`new_vesting_builder`]: enum.Recipient.html#method.new_vesting_builder
/// [`new_staking_builder`]: enum.Recipient.html#method.new_staking_builder
pub enum Recipient {
    Basic {
        address: Address,
    },
    HtlcCreation {
        data: HtlcCreationData,
    },
    VestingCreation {
        data: VestingCreationData,
    },
    Staking {
        address: Address,
        data: StakingTransaction,
    },
}

impl Recipient {
    /// Creates a basic `Recipient` (i.e., the recipient is a normal address and no contract).
    /// A basic recipient only consists of an address.
    ///
    /// # Examples
    ///
    /// ```
    /// use nimiq_transaction_builder::Recipient;
    /// use nimiq_keys::Address;
    ///
    /// let recipient = Recipient::new_basic(
    ///     Address::from_any_str("NQ25 B7NR A1HC V4R2 YRKD 20PR RPGS MNV7 D812").unwrap()
    /// );
    /// ```
    pub fn new_basic(address: Address) -> Self {
        Recipient::Basic { address }
    }

    /// Initiates a [`HtlcRecipientBuilder`] that can be used to create new HTLC contracts.
    /// The [`generate`] method of the builder will then return a `Recipient`.
    ///
    /// # Examples
    ///
    /// ```
    /// use nimiq_transaction_builder::Recipient;
    /// use nimiq_keys::Address;
    /// use nimiq_hash::{Blake2bHasher, Hasher, HashOutput};
    ///
    /// // Hash data for HTLC.
    /// // The actual pre_image must be a hash, so we have to hash our secret first.
    /// let secret = "supersecret";
    /// let pre_image = Blake2bHasher::default().digest(&secret.as_bytes());
    /// // To get the hash_root, we have to hash the pre_image multiple times.
    /// let hash_count = 10;
    /// let mut hash_root = pre_image;
    /// for _ in 0..hash_count {
    ///     hash_root = Blake2bHasher::default().digest(hash_root.as_bytes());
    /// }
    ///
    /// let mut recipient_builder = Recipient::new_htlc_builder();
    /// recipient_builder.with_sender(
    ///     Address::from_any_str("NQ25 B7NR A1HC V4R2 YRKD 20PR RPGS MNV7 D812").unwrap()
    /// );
    /// recipient_builder.with_recipient(
    ///     Address::from_any_str("NQ46 MNYU LQ93 GYYS P5DC YA51 L5JP UPUT KR62").unwrap()
    /// );
    /// recipient_builder.with_timeout_block(100)
    ///     .with_blake2b_hash(hash_root, hash_count);
    /// let recipient = recipient_builder.generate();
    /// assert!(recipient.is_ok());
    /// ```
    ///
    /// [`HtlcRecipientBuilder`]: htlc_contract/struct.HtlcRecipientBuilder.html
    /// [`generate`]: htlc_contract/struct.HtlcRecipientBuilder.html#method.generate
    pub fn new_htlc_builder() -> HtlcRecipientBuilder {
        HtlcRecipientBuilder::new()
    }

    /// Initiates a [`VestingRecipientBuilder`] that can be used to create new vesting contracts
    /// owned by the `owner` address.
    /// The [`generate`] method of the builder will then return a `Recipient`.
    ///
    /// # Examples
    ///
    /// ```
    /// use nimiq_transaction_builder::Recipient;
    /// use nimiq_keys::Address;
    /// use nimiq_primitives::coin::Coin;
    ///
    /// let owner = Address::from_any_str("NQ25 B7NR A1HC V4R2 YRKD 20PR RPGS MNV7 D812").unwrap();
    /// let mut recipient_builder = Recipient::new_vesting_builder(owner);
    /// recipient_builder.with_steps(
    ///     Coin::from_u64_unchecked(10_000), // total amount
    ///     13377, // start block
    ///     100, // every 100 blocks
    ///     5 // five steps
    /// );
    /// let recipient = recipient_builder.generate();
    /// assert!(recipient.is_ok());
    /// ```
    ///
    /// [`VestingRecipientBuilder`]: vesting_contract/struct.VestingRecipientBuilder.html
    /// [`generate`]: vesting_contract/struct.VestingRecipientBuilder.html#method.generate
    pub fn new_vesting_builder(owner: Address) -> VestingRecipientBuilder {
        VestingRecipientBuilder::new(owner)
    }

    /// Initiates a [`StakingRecipientBuilder`] that can be used to interact with the staking
    /// contract at address `staking_contract`.
    /// The [`generate`] method of the builder will then return a `Recipient`.
    ///
    /// # Examples
    ///
    /// ```
    /// use nimiq_transaction_builder::Recipient;
    /// use nimiq_keys::Address;
    /// use nimiq_bls::KeyPair;
    /// use nimiq_utils::key_rng::SecureGenerate;
    ///
    /// let validator_key_pair = KeyPair::generate_default_csprng();
    ///
    /// let staking_contract = Address::from_any_str("NQ25 B7NR A1HC V4R2 YRKD 20PR RPGS MNV7 D812").unwrap();
    /// let validator_key = validator_key_pair.public_key;
    /// let mut recipient_builder = Recipient::new_staking_builder(staking_contract);
    /// recipient_builder.stake(&validator_key, None);
    /// let recipient = recipient_builder.generate();
    /// assert!(recipient.is_some());
    /// ```
    ///
    /// [`StakingRecipientBuilder`]: staking_contract/struct.StakingRecipientBuilder.html
    /// [`generate`]: staking_contract/struct.StakingRecipientBuilder.html#method.generate
    pub fn new_staking_builder(staking_contract: Address) -> StakingRecipientBuilder {
        StakingRecipientBuilder::new(staking_contract)
    }

    /// This method checks whether the transaction is a contract creation.
    /// Vesting and HTLC recipients do create new contracts.
    /// Basic recipients and the staking contract do not create new contracts.
    pub fn is_creation(&self) -> bool {
        match self {
            Recipient::Basic { .. } | Recipient::Staking { .. } => false,
            _ => true,
        }
    }

    /// This method checks whether the transaction is a signalling transaction
    /// (i.e., requires a zero value and a special proof step).
    /// Only the following transactions on the staking contract are signalling transactions:
    /// * [`update validator details`]
    /// * [`retire validators`]
    /// * [`re-activate validators`]
    /// * [`unpark validators`]
    ///
    /// [`update validator details`]: staking_contract/struct.StakingRecipientBuilder.html#method.update_validator
    /// [`retire validators`]: staking_contract/struct.StakingRecipientBuilder.html#method.retire_validator
    /// [`re-activate validators`]: staking_contract/struct.StakingRecipientBuilder.html#method.reactivate_validator
    /// [`unpark validators`]: staking_contract/struct.StakingRecipientBuilder.html#method.unpark_validator
    pub fn is_signalling(&self) -> bool {
        match self {
            Recipient::Staking { data, .. } => data.is_signalling(),
            _ => false,
        }
    }

    /// Returns the account type of the recipient.
    pub fn account_type(&self) -> AccountType {
        match self {
            Recipient::Basic { .. } => AccountType::Basic,
            Recipient::HtlcCreation { .. } => AccountType::HTLC,
            Recipient::VestingCreation { .. } => AccountType::Vesting,
            Recipient::Staking { .. } => AccountType::Staking,
        }
    }

    /// Returns the recipient address if this is not a contract creation.
    pub fn address(&self) -> Option<&Address> {
        match self {
            Recipient::Basic { address } | Recipient::Staking { address, .. } => Some(address),
            _ => None,
        }
    }

    /// Returns the data field for the transaction.
    pub fn data(&self) -> Vec<u8> {
        match self {
            Recipient::Basic { .. } => Vec::new(),
            Recipient::HtlcCreation { data } => data.serialize_to_vec(),
            Recipient::VestingCreation { data } => data.serialize_to_vec(),
            Recipient::Staking { data, .. } => data.serialize_to_vec(),
        }
    }

    /// Validates the sender of the transaction.
    /// For certain transactions, the sender must equal the recipient.
    /// Currently, only the following transactions require this:
    /// * [`retire stake`]
    /// * [`re-activate stake`]
    ///
    /// [`retire stake`]: recipient/staking_contract/struct.StakingRecipientBuilder.html#method.retire_stake
    /// [`re-activate stake`]: recipient/staking_contract/struct.StakingRecipientBuilder.html#method.reactivate_stake
    pub fn is_valid_sender(&self, sender: &Address, sender_type: Option<AccountType>) -> bool {
        match self {
            Recipient::Staking { address, data } => {
                if data.is_self_transaction() {
                    address == sender && sender_type == Some(AccountType::Staking)
                } else {
                    true
                }
            }
            _ => true,
        }
    }
}
