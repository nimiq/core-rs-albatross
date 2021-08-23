use beserial::{Serialize, SerializingError, WriteBytesExt};
use hash::{Blake2bHash, Sha256Hash};
use keys::KeyPair;
use transaction::account::htlc_contract::{AnyHash, HashAlgorithm, ProofType};
use transaction::{SignatureProof, Transaction};

/// The `HtlcProof` represents a serializable form of all possible proof types
/// for a transaction from a HTLC contract.
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
#[derive(Clone, Debug)]
pub enum HtlcProof {
    RegularTransfer {
        hash_algorithm: HashAlgorithm,
        hash_depth: u8,
        hash_root: AnyHash,
        pre_image: AnyHash,
        recipient_signature: SignatureProof,
    },
    EarlyResolve {
        recipient_signature: SignatureProof,
        sender_signature: SignatureProof,
    },
    TimeoutResolve {
        signature: SignatureProof,
    },
}

impl Serialize for HtlcProof {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        let mut size = 0;
        match self {
            HtlcProof::RegularTransfer {
                hash_algorithm,
                hash_depth,
                hash_root,
                pre_image,
                recipient_signature,
            } => {
                size += ProofType::RegularTransfer.serialize(writer)?;
                size += hash_algorithm.serialize(writer)?;
                size += hash_depth.serialize(writer)?;
                size += hash_root.serialize(writer)?;
                size += pre_image.serialize(writer)?;
                size += recipient_signature.serialize(writer)?;
            }
            HtlcProof::EarlyResolve {
                recipient_signature,
                sender_signature,
            } => {
                size += ProofType::EarlyResolve.serialize(writer)?;
                size += recipient_signature.serialize(writer)?;
                size += sender_signature.serialize(writer)?;
            }
            HtlcProof::TimeoutResolve { signature } => {
                size += ProofType::TimeoutResolve.serialize(writer)?;
                size += signature.serialize(writer)?;
            }
        }
        Ok(size)
    }

    fn serialized_size(&self) -> usize {
        let mut size = 0;
        match self {
            HtlcProof::RegularTransfer {
                hash_algorithm,
                hash_depth,
                hash_root,
                pre_image,
                recipient_signature,
            } => {
                size += ProofType::RegularTransfer.serialized_size();
                size += hash_algorithm.serialized_size();
                size += hash_depth.serialized_size();
                size += hash_root.serialized_size();
                size += pre_image.serialized_size();
                size += recipient_signature.serialized_size();
            }
            HtlcProof::EarlyResolve {
                recipient_signature,
                sender_signature,
            } => {
                size += ProofType::EarlyResolve.serialized_size();
                size += recipient_signature.serialized_size();
                size += sender_signature.serialized_size();
            }
            HtlcProof::TimeoutResolve { signature } => {
                size += ProofType::TimeoutResolve.serialized_size();
                size += signature.serialized_size();
            }
        }
        size
    }
}

/// The `HtlcProofBuilder` can be used to build proofs for transactions
/// that originate in a HTLC contract.
#[derive(Clone, Debug)]
pub struct HtlcProofBuilder {
    pub transaction: Transaction,
    proof: Option<HtlcProof>,
}

impl HtlcProofBuilder {
    /// Creates a new `HtlcProofBuilder` from a `transaction`.
    pub fn new(transaction: Transaction) -> Self {
        HtlcProofBuilder {
            transaction,
            proof: None,
        }
    }

    /// This helper method can be used to generate the `SignatureProof` as required by all
    /// other methods in this builder.
    ///
    /// It does not modify the builder's state and is to be used in combination.
    ///
    /// # Examples
    ///
    /// ```
    /// # use nimiq_keys::{Address, KeyPair};
    /// use nimiq_transaction_builder::{Recipient, TransactionBuilder};
    /// use nimiq_hash::{Blake2bHasher, Hasher, HashOutput};
    /// use nimiq_primitives::coin::Coin;
    /// use nimiq_primitives::networks::NetworkId;
    /// use nimiq_primitives::account::AccountType;
    /// # use nimiq_utils::key_rng::SecureGenerate;
    ///
    /// # let key_pair_sender = KeyPair::generate_default_csprng();
    /// # let key_pair_recipient = KeyPair::generate_default_csprng();
    ///
    /// let sender_address = Address::from_any_str("NQ46 MNYU LQ93 GYYS P5DC YA51 L5JP UPUT KR62").unwrap();
    /// let recipient = Recipient::new_basic(
    ///     Address::from(&key_pair_recipient)
    /// );
    ///
    /// let mut builder = TransactionBuilder::with_required(
    ///     sender_address,
    ///     recipient,
    ///     Coin::from_u64_unchecked(100),
    ///     1,
    ///     NetworkId::Main
    /// );
    /// builder.with_sender_type(AccountType::HTLC);
    ///
    /// let proof_builder = builder.generate().unwrap();
    /// let mut htlc_proof_builder = proof_builder.unwrap_htlc();
    ///
    /// let signature_sender = htlc_proof_builder.signature_with_key_pair(&key_pair_sender);
    /// let signature_recipient = htlc_proof_builder.signature_with_key_pair(&key_pair_recipient);
    /// htlc_proof_builder.early_resolve(signature_sender, signature_recipient);
    ///
    /// let final_transaction = htlc_proof_builder.generate();
    /// assert!(final_transaction.is_some());
    /// assert!(final_transaction.unwrap().verify(NetworkId::Main).is_ok());
    /// ```
    pub fn signature_with_key_pair(&self, key_pair: &KeyPair) -> SignatureProof {
        let signature = key_pair.sign(self.transaction.serialize_content().as_slice());
        SignatureProof::from(key_pair.public, signature)
    }

    /// This method creates a proof for the `TimeoutResolve` case, i.e.,
    /// after a blockchain height called `timeout` is reached, the `sender` can withdraw the funds.
    ///
    /// The required signature can be generated using [`signature_with_key_pair`].
    ///
    /// # Examples
    ///
    /// ```
    /// # use nimiq_keys::{Address, KeyPair};
    /// use nimiq_transaction_builder::{Recipient, TransactionBuilder};
    /// use nimiq_hash::{Blake2bHasher, Hasher, HashOutput};
    /// use nimiq_primitives::coin::Coin;
    /// use nimiq_primitives::networks::NetworkId;
    /// use nimiq_primitives::account::AccountType;
    /// # use nimiq_utils::key_rng::SecureGenerate;
    ///
    /// # let key_pair = KeyPair::generate_default_csprng();
    ///
    /// let sender_address = Address::from_any_str("NQ46 MNYU LQ93 GYYS P5DC YA51 L5JP UPUT KR62").unwrap();
    /// let recipient = Recipient::new_basic(
    ///     Address::from(&key_pair)
    /// );
    ///
    /// let mut builder = TransactionBuilder::with_required(
    ///     sender_address,
    ///     recipient,
    ///     Coin::from_u64_unchecked(100),
    ///     1,
    ///     NetworkId::Main
    /// );
    /// builder.with_sender_type(AccountType::HTLC);
    ///
    /// let proof_builder = builder.generate().unwrap();
    /// let mut htlc_proof_builder = proof_builder.unwrap_htlc();
    ///
    /// let signature_sender = htlc_proof_builder.signature_with_key_pair(&key_pair);
    /// htlc_proof_builder.timeout_resolve(signature_sender);
    ///
    /// let final_transaction = htlc_proof_builder.generate();
    /// assert!(final_transaction.is_some());
    /// assert!(final_transaction.unwrap().verify(NetworkId::Main).is_ok());
    /// ```
    ///
    /// [`signature_with_key_pair`]: struct.HtlcProofBuilder.html#method.signature_with_key_pair
    pub fn timeout_resolve(&mut self, sender_signature: SignatureProof) -> &mut Self {
        self.proof = Some(HtlcProof::TimeoutResolve {
            signature: sender_signature,
        });
        self
    }

    /// This method creates a proof for the `EarlyResolve` case, i.e.,
    /// if both `sender` and `recipient` sign the transaction, the funds can be withdrawn at any time.
    ///
    /// The required signatures can be generated using [`signature_with_key_pair`].
    ///
    /// # Examples
    ///
    /// ```
    /// # use nimiq_keys::{Address, KeyPair};
    /// use nimiq_transaction_builder::{Recipient, TransactionBuilder};
    /// use nimiq_hash::{Blake2bHasher, Hasher, HashOutput};
    /// use nimiq_primitives::coin::Coin;
    /// use nimiq_primitives::networks::NetworkId;
    /// use nimiq_primitives::account::AccountType;
    /// # use nimiq_utils::key_rng::SecureGenerate;
    ///
    /// # let key_pair_sender = KeyPair::generate_default_csprng();
    /// # let key_pair_recipient = KeyPair::generate_default_csprng();
    ///
    /// let sender_address = Address::from_any_str("NQ46 MNYU LQ93 GYYS P5DC YA51 L5JP UPUT KR62").unwrap();
    /// let recipient = Recipient::new_basic(
    ///     Address::from(&key_pair_recipient)
    /// );
    ///
    /// let mut builder = TransactionBuilder::with_required(
    ///     sender_address,
    ///     recipient,
    ///     Coin::from_u64_unchecked(100),
    ///     1,
    ///     NetworkId::Main
    /// );
    /// builder.with_sender_type(AccountType::HTLC);
    ///
    /// let proof_builder = builder.generate().unwrap();
    /// let mut htlc_proof_builder = proof_builder.unwrap_htlc();
    ///
    /// let signature_sender = htlc_proof_builder.signature_with_key_pair(&key_pair_sender);
    /// let signature_recipient = htlc_proof_builder.signature_with_key_pair(&key_pair_recipient);
    /// htlc_proof_builder.early_resolve(signature_sender, signature_recipient);
    ///
    /// let final_transaction = htlc_proof_builder.generate();
    /// assert!(final_transaction.is_some());
    /// assert!(final_transaction.unwrap().verify(NetworkId::Main).is_ok());
    /// ```
    ///
    /// [`signature_with_key_pair`]: struct.HtlcProofBuilder.html#method.signature_with_key_pair
    pub fn early_resolve(
        &mut self,
        sender_signature: SignatureProof,
        recipient_signature: SignatureProof,
    ) -> &mut Self {
        self.proof = Some(HtlcProof::EarlyResolve {
            sender_signature,
            recipient_signature,
        });
        self
    }

    /// This method creates a proof for the `RegularTransfer` case.
    ///
    /// The contract stores a `hash_root`. The `recipient` can withdraw the funds before the
    /// `timeout` has been reached by presenting a hash that will yield the `hash_root`
    /// when re-hashing it `hash_count` times.
    /// By presenting a hash that will yield the `hash_root` after re-hashing it k < `hash_count`
    /// times, the `recipient` can retrieve 1/k of the funds.
    ///
    /// The required signature can be generated using [`signature_with_key_pair`].
    ///
    /// [`signature_with_key_pair`]: struct.HtlcProofBuilder.html#method.signature_with_key_pair
    pub fn regular_transfer(
        &mut self,
        hash_algorithm: HashAlgorithm,
        pre_image: AnyHash,
        hash_count: u8,
        hash_root: AnyHash,
        recipient_signature: SignatureProof,
    ) -> &mut Self {
        self.proof = Some(HtlcProof::RegularTransfer {
            hash_algorithm,
            hash_depth: hash_count,
            hash_root,
            pre_image,
            recipient_signature,
        });
        self
    }

    /// This method creates a proof for the `RegularTransfer` case using Sha256 hashes.
    ///
    /// The contract stores a `hash_root`. The `recipient` can withdraw the funds before the
    /// `timeout` has been reached by presenting a hash that will yield the `hash_root`
    /// when re-hashing it `hash_count` times.
    /// By presenting a hash that will yield the `hash_root` after re-hashing it k < `hash_count`
    /// times, the `recipient` can retrieve 1/k of the funds.
    ///
    /// The required signature can be generated using [`signature_with_key_pair`].
    ///
    /// # Examples
    ///
    /// ```
    /// # use nimiq_keys::{Address, KeyPair};
    /// use nimiq_transaction_builder::{Recipient, TransactionBuilder};
    /// use nimiq_hash::{Sha256Hasher, Hasher, HashOutput};
    /// use nimiq_primitives::coin::Coin;
    /// use nimiq_primitives::networks::NetworkId;
    /// use nimiq_primitives::account::AccountType;
    /// # use nimiq_utils::key_rng::SecureGenerate;
    ///
    /// # let key_pair = KeyPair::generate_default_csprng();
    ///
    /// // Hash data for HTLC.
    /// // The actual pre_image must be a hash, so we have to hash our secret first.
    /// let secret = "supersecret";
    /// let pre_image = Sha256Hasher::default().digest(&secret.as_bytes());
    /// // To get the hash_root, we have to hash the pre_image multiple times.
    /// let hash_count = 10;
    /// let mut hash_root = pre_image.clone();
    /// for _ in 0..hash_count {
    ///     hash_root = Sha256Hasher::default().digest(hash_root.as_bytes());
    /// }
    ///
    /// let sender_address = Address::from_any_str("NQ46 MNYU LQ93 GYYS P5DC YA51 L5JP UPUT KR62").unwrap();
    /// let recipient = Recipient::new_basic(
    ///     Address::from(&key_pair)
    /// );
    ///
    /// let mut builder = TransactionBuilder::with_required(
    ///     sender_address,
    ///     recipient,
    ///     Coin::from_u64_unchecked(100),
    ///     1,
    ///     NetworkId::Main
    /// );
    /// builder.with_sender_type(AccountType::HTLC);
    ///
    /// let proof_builder = builder.generate().unwrap();
    /// let mut htlc_proof_builder = proof_builder.unwrap_htlc();
    ///
    /// let signature = htlc_proof_builder.signature_with_key_pair(&key_pair);
    /// htlc_proof_builder.regular_transfer_sha256(pre_image, hash_count, hash_root, signature);
    ///
    /// let final_transaction = htlc_proof_builder.generate();
    /// assert!(final_transaction.is_some());
    /// assert!(final_transaction.unwrap().verify(NetworkId::Main).is_ok());
    /// ```
    ///
    /// [`signature_with_key_pair`]: struct.HtlcProofBuilder.html#method.signature_with_key_pair
    pub fn regular_transfer_sha256(
        &mut self,
        pre_image: Sha256Hash,
        hash_count: u8,
        hash_root: Sha256Hash,
        recipient_signature: SignatureProof,
    ) -> &mut Self {
        let pre_image: [u8; 32] = pre_image.into();
        let hash_root: [u8; 32] = hash_root.into();
        self.regular_transfer(
            HashAlgorithm::Sha256,
            pre_image.into(),
            hash_count,
            hash_root.into(),
            recipient_signature,
        )
    }

    /// This method creates a proof for the `RegularTransfer` case using Blake2b hashes.
    ///
    /// The contract stores a `hash_root`. The `recipient` can withdraw the funds before the
    /// `timeout` has been reached by presenting a hash that will yield the `hash_root`
    /// when re-hashing it `hash_count` times.
    /// By presenting a hash that will yield the `hash_root` after re-hashing it k < `hash_count`
    /// times, the `recipient` can retrieve 1/k of the funds.
    ///
    /// The required signature can be generated using [`signature_with_key_pair`].
    ///
    /// # Examples
    ///
    /// ```
    /// # use nimiq_keys::{Address, KeyPair};
    /// use nimiq_transaction_builder::{Recipient, TransactionBuilder};
    /// use nimiq_hash::{Blake2bHasher, Hasher, HashOutput};
    /// use nimiq_primitives::coin::Coin;
    /// use nimiq_primitives::networks::NetworkId;
    /// use nimiq_primitives::account::AccountType;
    /// # use nimiq_utils::key_rng::SecureGenerate;
    ///
    /// # let key_pair = KeyPair::generate_default_csprng();
    ///
    /// // Hash data for HTLC.
    /// // The actual pre_image must be a hash, so we have to hash our secret first.
    /// let secret = "supersecret";
    /// let pre_image = Blake2bHasher::default().digest(&secret.as_bytes());
    /// // To get the hash_root, we have to hash the pre_image multiple times.
    /// let hash_count = 10;
    /// let mut hash_root = pre_image.clone();
    /// for _ in 0..hash_count {
    ///     hash_root = Blake2bHasher::default().digest(hash_root.as_bytes());
    /// }
    ///
    /// let sender_address = Address::from_any_str("NQ46 MNYU LQ93 GYYS P5DC YA51 L5JP UPUT KR62").unwrap();
    /// let recipient = Recipient::new_basic(
    ///     Address::from(&key_pair)
    /// );
    ///
    /// let mut builder = TransactionBuilder::with_required(
    ///     sender_address,
    ///     recipient,
    ///     Coin::from_u64_unchecked(100),
    ///     1,
    ///     NetworkId::Main
    /// );
    /// builder.with_sender_type(AccountType::HTLC);
    ///
    /// let proof_builder = builder.generate().unwrap();
    /// let mut htlc_proof_builder = proof_builder.unwrap_htlc();
    ///
    /// let signature = htlc_proof_builder.signature_with_key_pair(&key_pair);
    /// htlc_proof_builder.regular_transfer_blake2b(pre_image, hash_count, hash_root, signature);
    ///
    /// let final_transaction = htlc_proof_builder.generate();
    /// assert!(final_transaction.is_some());
    /// assert!(final_transaction.unwrap().verify(NetworkId::Main).is_ok());
    /// ```
    ///
    /// [`signature_with_key_pair`]: struct.HtlcProofBuilder.html#method.signature_with_key_pair
    pub fn regular_transfer_blake2b(
        &mut self,
        pre_image: Blake2bHash,
        hash_count: u8,
        hash_root: Blake2bHash,
        recipient_signature: SignatureProof,
    ) -> &mut Self {
        let pre_image: [u8; 32] = pre_image.into();
        let hash_root: [u8; 32] = hash_root.into();
        self.regular_transfer(
            HashAlgorithm::Blake2b,
            pre_image.into(),
            hash_count,
            hash_root.into(),
            recipient_signature,
        )
    }

    /// This method generates the final transaction if the signature has been set correctly.
    /// Otherwise, it returns `None`.
    pub fn generate(self) -> Option<Transaction> {
        let mut tx = self.transaction;
        tx.proof = self.proof?.serialize_to_vec();
        Some(tx)
    }
}
