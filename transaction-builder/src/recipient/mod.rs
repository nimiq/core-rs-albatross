use nimiq_keys::Address;
use nimiq_primitives::{account::AccountType, policy::Policy};
use nimiq_serde::Serialize;
use nimiq_transaction::account::{
    htlc_contract::CreationTransactionData as HtlcCreationData,
    staking_contract::IncomingStakingTransactionData,
    vesting_contract::CreationTransactionData as VestingCreationData,
};

use crate::recipient::{
    htlc_contract::HtlcRecipientBuilder, staking_contract::StakingRecipientBuilder,
    vesting_contract::VestingRecipientBuilder,
};

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
#[derive(Clone, Debug)]
pub enum Recipient {
    Basic {
        address: Address,
        data: Vec<u8>,
    },
    HtlcCreation {
        data: HtlcCreationData,
    },
    VestingCreation {
        data: VestingCreationData,
    },
    Staking {
        data: IncomingStakingTransactionData,
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
        Recipient::Basic {
            address,
            data: vec![],
        }
    }

    pub fn new_basic_with_data(address: Address, data: Vec<u8>) -> Self {
        Recipient::Basic { address, data }
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
    /// recipient_builder.with_timeout(100)
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
    /// let staker_address: Address = [0;20].into();
    ///
    /// let mut recipient_builder = Recipient::new_staking_builder();
    /// recipient_builder.stake(staker_address);
    /// let recipient = recipient_builder.generate();
    /// assert!(recipient.is_some());
    /// ```
    ///
    /// [`StakingRecipientBuilder`]: staking_contract/struct.StakingRecipientBuilder.html
    /// [`generate`]: staking_contract/struct.StakingRecipientBuilder.html#method.generate
    pub fn new_staking_builder() -> StakingRecipientBuilder {
        StakingRecipientBuilder::new()
    }

    /// This method checks whether the transaction is a contract creation.
    /// Vesting and HTLC recipients do create new contracts.
    /// Basic recipients and the staking contract do not create new contracts.
    pub fn is_creation(&self) -> bool {
        matches!(
            self,
            Recipient::HtlcCreation { .. } | Recipient::VestingCreation { .. }
        )
    }

    /// This method checks whether the transaction is a signaling transaction
    /// (i.e., requires a zero value).
    /// Only the following transactions on the staking contract are signaling transactions:
    /// * [`update validator`]
    /// * [`retire validator`]
    /// * [`re-activate validator`]
    /// * [`update staker`]
    /// * [`retire staker`]
    /// * [`re-activate staker`]
    pub fn is_signaling(&self) -> bool {
        match self {
            Recipient::Staking { data } => data.is_signaling(),
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
    pub fn address(&self) -> Option<Address> {
        match self {
            Recipient::Basic { address, .. } => Some(address.clone()),
            Recipient::Staking { .. } => Some(Policy::STAKING_CONTRACT_ADDRESS),
            _ => None,
        }
    }

    /// Returns the data field for the transaction.
    pub fn data(&self) -> Vec<u8> {
        match self {
            Recipient::Basic { data, .. } => data.clone(),
            Recipient::HtlcCreation { data } => data.serialize_to_vec(),
            Recipient::VestingCreation { data } => data.to_tx_data(),
            Recipient::Staking { data } => data.serialize_to_vec(),
        }
    }
}
