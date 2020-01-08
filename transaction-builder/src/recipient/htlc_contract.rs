use failure::Fail;

use hash::{Blake2bHash, Sha256Hash};
use keys::Address;
use transaction::account::htlc_contract::{AnyHash, HashAlgorithm};
use transaction::account::htlc_contract::CreationTransactionData as HtlcCreationData;

use crate::recipient::Recipient;

/// Building a HTLC recipient can fail if mandatory fields are not set.
/// In these cases, a `HtlcRecipientBuilderError` is returned.
#[derive(Debug, Fail)]
pub enum HtlcRecipientBuilderError {
    /// The `sender` field of the [`HtlcRecipientBuilder`] has not been set.
    /// Call [`with_sender`] to set this field.
    ///
    /// [`HtlcRecipientBuilder`]: struct.HtlcRecipientBuilder.html
    /// [`with_sender`]: struct.HtlcRecipientBuilder.html#method.with_sender
    #[fail(display = "The HTLC sender address is missing.")]
    NoSender,
    /// The `recipient` field of the [`HtlcRecipientBuilder`] has not been set.
    /// Call [`with_recipient`] to set this field.
    ///
    /// [`HtlcRecipientBuilder`]: struct.HtlcRecipientBuilder.html
    /// [`with_recipient`]: struct.HtlcRecipientBuilder.html#method.with_recipient
    #[fail(display = "The HTLC recipient address is missing.")]
    NoRecipient,
    /// The hash data of the [`HtlcRecipientBuilder`] has not been set.
    /// Call [`with_hash`], [`with_sha256_hash`], or [`with_blake2b_hash`] to set this field.
    ///
    /// [`HtlcRecipientBuilder`]: struct.HtlcRecipientBuilder.html
    /// [`with_hash`]: struct.HtlcRecipientBuilder.html#method.with_hash
    /// [`with_sha256_hash`]: struct.HtlcRecipientBuilder.html#method.with_sha256_hash
    /// [`with_blake2b_hash`]: struct.HtlcRecipientBuilder.html#method.with_blake2b_hash
    #[fail(display = "The HTLC hash data is missing.")]
    NoHash,
    /// The `timeout` field of the [`HtlcRecipientBuilder`] has not been set.
    /// Call [`with_timeout`] to set this field.
    ///
    /// [`HtlcRecipientBuilder`]: struct.HtlcRecipientBuilder.html
    /// [`with_timeout`]: struct.HtlcRecipientBuilder.html#method.with_timeout
    #[fail(display = "The HTLC's timeout is missing.")]
    NoTimeout,
}

/// A `HtlcRecipientBuilder` can be used to create new HTLC contracts.
/// A HTLC contract locks funds between two parties called `sender` and `recipient`.
/// The sender does not necessarily needs to coincide with the transaction's sender.
///
/// The funds can be unlocked by one of three mechanisms:
/// 1. After a blockchain height called `timeout` is reached, the `sender` can withdraw the funds.
///     (called `TimeoutResolve`)
/// 2. The contract stores a `hash_root`. The `recipient` can withdraw the funds before the
///     `timeout` has been reached by presenting a hash that will yield the `hash_root`
///     when re-hashing it `hash_count` times.
///     By presenting a hash that will yield the `hash_root` after re-hashing it k < `hash_count`
///     times, the `recipient` can retrieve 1/k of the funds.
///     (called `RegularTransfer`)
/// 3. If both `sender` and `recipient` sign the transaction, the funds can be withdrawn at any time.
///     (called `EarlyResolve`)
#[derive(Default)]
pub struct HtlcRecipientBuilder {
    sender: Option<Address>,
    recipient: Option<Address>,
    hash_algorithm: Option<HashAlgorithm>,
    hash_root: Option<AnyHash>,
    hash_count: u8,
    timeout: Option<u32>,
}

impl HtlcRecipientBuilder {
    /// Creates an empty HTLC contract with no parameters set.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a HTLC contract with all required parameters set.
    /// This contract can be fully resolved by the `recipient` before the `timeout_block`
    /// by presenting a sha256 hash that, when re-hashed, yields `hashed_secret`.
    pub fn new_single_sha256(sender: Address, recipient: Address, timeout_block: u32, hashed_secret: Sha256Hash) -> Self {
        let mut builder = Self::new();
        builder.with_sender(sender)
            .with_recipient(recipient)
            .with_sha256_hash(hashed_secret, 1)
            .with_timeout_block(timeout_block);
        builder
    }

    /// Sets the `sender` field of the HTLC.
    pub fn with_sender(&mut self, sender: Address) -> &mut Self {
        self.sender = Some(sender);
        self
    }

    /// Sets the `recipient` field of the HTLC.
    pub fn with_recipient(&mut self, recipient: Address) -> &mut Self {
        self.recipient = Some(recipient);
        self
    }

    /// Sets the hash data for the HTLC.
    /// The `hash_root` is the result of hashing the pre-image hash `hash_count` times.
    pub fn with_hash(&mut self, hash_root: AnyHash, hash_count: u8, hash_algorithm: HashAlgorithm) -> &mut Self {
        self.hash_root = Some(hash_root);
        self.hash_count = hash_count;
        self.hash_algorithm = Some(hash_algorithm);
        self
    }

    /// Sets the hash data for the HTLC using Sha256 hashes.
    /// The `hash_root` is the result of hashing the pre-image hash `hash_count` times.
    ///
    /// # Examples
    ///
    /// ```
    /// use nimiq_transaction_builder::recipient::htlc_contract::HtlcRecipientBuilder;
    /// use nimiq_hash::{Sha256Hasher, Hasher, HashOutput};
    ///
    /// // Hash data for HTLC.
    /// // The actual pre_image must be a hash, so we have to hash our secret first.
    /// let secret = "supersecret";
    /// let pre_image = Sha256Hasher::default().digest(&secret.as_bytes());
    /// // To get the hash_root, we have to hash the pre_image multiple times.
    /// let hash_count = 10;
    /// let mut hash_root = pre_image;
    /// for _ in 0..hash_count {
    ///     hash_root = Sha256Hasher::default().digest(hash_root.as_bytes());
    /// }
    ///
    /// let mut recipient_builder = HtlcRecipientBuilder::new();
    /// recipient_builder.with_sha256_hash(hash_root, hash_count);
    /// ```
    pub fn with_sha256_hash(&mut self, hash_root: Sha256Hash, hash_count: u8) -> &mut Self {
        let hash: [u8; 32] = hash_root.into();
        self.hash_root = Some(AnyHash::from(hash));
        self.hash_count = hash_count;
        self.hash_algorithm = Some(HashAlgorithm::Sha256);
        self
    }

    /// Sets the hash data for the HTLC using Blake2b hashes.
    /// The `hash_root` is the result of hashing the pre-image hash `hash_count` times.
    ///
    /// # Examples
    ///
    /// ```
    /// use nimiq_transaction_builder::recipient::htlc_contract::HtlcRecipientBuilder;
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
    /// let mut recipient_builder = HtlcRecipientBuilder::new();
    /// recipient_builder.with_blake2b_hash(hash_root, hash_count);
    /// ```
    pub fn with_blake2b_hash(&mut self, hash_root: Blake2bHash, hash_count: u8) -> &mut Self {
        let hash: [u8; 32] = hash_root.into();
        self.hash_root = Some(AnyHash::from(hash));
        self.hash_count = hash_count;
        self.hash_algorithm = Some(HashAlgorithm::Blake2b);
        self
    }

    /// Sets the blockchain height at which the `sender` automatically gains control over the funds.
    pub fn with_timeout_block(&mut self, timeout_block: u32) -> &mut Self {
        self.timeout = Some(timeout_block);
        self
    }

    /// This method tries putting together the contract creation,
    /// returning a [`Recipient`] in case of success.
    /// In case of a failure, it returns a [`HtlcRecipientBuilderError`].
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
    /// [`Recipient`]: ../enum.Recipient.html
    /// [`HtlcRecipientBuilderError`]: enum.HtlcRecipientBuilderError.html
    pub fn generate(self) -> Result<Recipient, HtlcRecipientBuilderError> {
        Ok(Recipient::HtlcCreation {
            data: HtlcCreationData {
                sender: self.sender.ok_or(HtlcRecipientBuilderError::NoSender)?,
                recipient: self.recipient.ok_or(HtlcRecipientBuilderError::NoRecipient)?,
                hash_algorithm: self.hash_algorithm.ok_or(HtlcRecipientBuilderError::NoHash)?,
                hash_root: self.hash_root.ok_or(HtlcRecipientBuilderError::NoHash)?,
                hash_count: self.hash_count,
                timeout: self.timeout.ok_or(HtlcRecipientBuilderError::NoTimeout)?
            }
        })
    }
}
