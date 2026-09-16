use nimiq_keys::Address;
use nimiq_primitives::{account::AccountType, policy::Policy};
use nimiq_serde::Serialize;
use nimiq_transaction::{
    account::{
        bridge_contract::CreationTransactionData as BridgeCreationData,
        htlc_contract::{AnyHash, CreationTransactionData as HtlcCreationData},
        oracle_contract::{
            CreationTransactionData as OracleCreationData, IncomingOracleTransactionData,
        },
        staking_contract::IncomingStakingTransactionData,
        vesting_contract::CreationTransactionData as VestingCreationData,
    },
    SignatureProof,
};

use crate::recipient::{
    bridge_contract::BridgeRecipientBuilder, htlc_contract::HtlcRecipientBuilder,
    oracle_contract::OracleRecipientBuilder, staking_contract::StakingRecipientBuilder,
    vesting_contract::VestingRecipientBuilder,
};

pub mod bridge_contract;
pub mod htlc_contract;
pub mod oracle_contract;
pub mod staking_contract;
pub mod vesting_contract;

/// A `Recipient` describes the recipient of a transaction.
/// This also determines the data field of the transaction to be built.
///
/// New contracts can be created using dedicated builders as described below.
///
/// There are eight types of recipients:
/// - basic recipients that can be built with [`new_basic`]
/// - HTLC contracts that can be set up with a builder using [`new_htlc_builder`]
/// - vesting contracts that can be set up with a builder using [`new_vesting_builder`]
/// - actions on the staking contract that built with [`new_staking_builder`]
/// - bridge contracts that can be set up with a builder using [`new_bridge_builder`]
/// - deposits into an existing bridge contract built with [`new_bridge_deposit`]
/// - oracle contracts that can be set up with a builder using [`new_oracle_builder`]
/// - actions on an existing oracle contract built with [`new_oracle_update`] and
///   [`new_oracle_change_owner`]
///
/// [`new_basic`]: enum.Recipient.html#method.new_basic
/// [`new_htlc_builder`]: enum.Recipient.html#method.new_htlc_builder
/// [`new_vesting_builder`]: enum.Recipient.html#method.new_vesting_builder
/// [`new_staking_builder`]: enum.Recipient.html#method.new_staking_builder
/// [`new_bridge_builder`]: enum.Recipient.html#method.new_bridge_builder
/// [`new_bridge_deposit`]: enum.Recipient.html#method.new_bridge_deposit
/// [`new_oracle_builder`]: enum.Recipient.html#method.new_oracle_builder
/// [`new_oracle_update`]: enum.Recipient.html#method.new_oracle_update
/// [`new_oracle_change_owner`]: enum.Recipient.html#method.new_oracle_change_owner
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
    BridgeCreation {
        data: BridgeCreationData,
    },
    Bridge {
        address: Address,
        data: Vec<u8>,
    },
    OracleCreation {
        data: OracleCreationData,
    },
    Oracle {
        address: Address,
        data: IncomingOracleTransactionData,
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

    /// Initiates a [`BridgeRecipientBuilder`] that can be used to create new bridge contracts.
    /// The [`generate`] method of the builder will then return a `Recipient`.
    ///
    /// [`BridgeRecipientBuilder`]: bridge_contract/struct.BridgeRecipientBuilder.html
    /// [`generate`]: bridge_contract/struct.BridgeRecipientBuilder.html#method.generate
    pub fn new_bridge_builder() -> BridgeRecipientBuilder {
        BridgeRecipientBuilder::new()
    }

    /// Initiates an [`OracleRecipientBuilder`] that can be used to create new oracle contracts
    /// owned by the `owner` address.
    /// The [`generate`] method of the builder will then return a `Recipient`.
    ///
    /// [`OracleRecipientBuilder`]: oracle_contract/struct.OracleRecipientBuilder.html
    /// [`generate`]: oracle_contract/struct.OracleRecipientBuilder.html#method.generate
    pub fn new_oracle_builder(owner: Address) -> OracleRecipientBuilder {
        OracleRecipientBuilder::with_owner_init(owner)
    }

    /// Creates a `Recipient` that deposits funds into the existing bridge contract at `address`,
    /// locking them for transfer to the bridge's destination chain.
    ///
    /// The bridge contract does not interpret `data`; it is carried in the transaction for the
    /// off-chain relayer, which reads the destination of the deposit from it.
    pub fn new_bridge_deposit(address: Address, data: Vec<u8>) -> Self {
        Recipient::Bridge { address, data }
    }

    /// Creates a `Recipient` that appends `hashes` to the existing oracle contract at `address`.
    ///
    /// This is a signaling transaction that must be signed by the oracle owner using an
    /// [`OracleDataBuilder`].
    ///
    /// [`OracleDataBuilder`]: crate::proof::oracle_contract::OracleDataBuilder
    pub fn new_oracle_update(address: Address, hashes: Vec<AnyHash>) -> Self {
        Recipient::Oracle {
            address,
            data: IncomingOracleTransactionData::Update {
                hashes,
                proof: SignatureProof::default(),
            },
        }
    }

    /// Creates a `Recipient` that transfers ownership of the existing oracle contract at
    /// `address` to `new_owner`.
    ///
    /// This is a signaling transaction that must be signed by the current oracle owner using an
    /// [`OracleDataBuilder`].
    ///
    /// [`OracleDataBuilder`]: crate::proof::oracle_contract::OracleDataBuilder
    pub fn new_oracle_change_owner(address: Address, new_owner: Address) -> Self {
        Recipient::Oracle {
            address,
            data: IncomingOracleTransactionData::ChangeOwner {
                new_owner,
                proof: SignatureProof::default(),
            },
        }
    }

    /// This method checks whether the transaction is a contract creation.
    /// Vesting, HTLC, Bridge, and Oracle recipients do create new contracts.
    /// Basic recipients and the staking contract do not create new contracts.
    pub fn is_creation(&self) -> bool {
        matches!(
            self,
            Recipient::HtlcCreation { .. }
                | Recipient::VestingCreation { .. }
                | Recipient::BridgeCreation { .. }
                | Recipient::OracleCreation { .. }
        )
    }

    /// This method checks whether the transaction is a signaling transaction
    /// (i.e., requires a zero value).
    /// All transactions to an existing oracle contract are signaling transactions, as are the
    /// following transactions on the staking contract:
    /// * [`update validator`]
    /// * [`retire validator`]
    /// * [`re-activate validator`]
    /// * [`update staker`]
    /// * [`retire staker`]
    /// * [`re-activate staker`]
    pub fn is_signaling(&self) -> bool {
        match self {
            Recipient::Staking { data } => data.is_signaling(),
            Recipient::Oracle { .. } => true,
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
            Recipient::BridgeCreation { .. } | Recipient::Bridge { .. } => AccountType::Bridge,
            Recipient::OracleCreation { .. } | Recipient::Oracle { .. } => AccountType::Oracle,
        }
    }

    /// Returns the recipient address if this is not a contract creation.
    pub fn address(&self) -> Option<Address> {
        match self {
            Recipient::Basic { address, .. }
            | Recipient::Bridge { address, .. }
            | Recipient::Oracle { address, .. } => Some(address.clone()),
            Recipient::Staking { .. } => Some(Policy::STAKING_CONTRACT_ADDRESS),
            _ => None,
        }
    }

    /// Returns the data field for the transaction.
    pub fn data(&self) -> Vec<u8> {
        match self {
            Recipient::Basic { data, .. } | Recipient::Bridge { data, .. } => data.clone(),
            Recipient::HtlcCreation { data } => data.serialize_to_vec(),
            Recipient::VestingCreation { data } => data.to_tx_data(),
            Recipient::Staking { data } => data.serialize_to_vec(),
            Recipient::BridgeCreation { data } => data.serialize_to_vec(),
            Recipient::OracleCreation { data } => data.serialize_to_vec(),
            Recipient::Oracle { data, .. } => data.serialize_to_vec(),
        }
    }
}
