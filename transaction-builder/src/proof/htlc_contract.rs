use nimiq_hash::{Blake2bHash, Sha256Hash};
use nimiq_keys::KeyPair;
use nimiq_serde::Serialize;
use nimiq_transaction::{
    account::htlc_contract::{AnyHash, OutgoingHTLCTransactionProof, PreImage},
    EdDSASignatureProof, SignatureProof, Transaction,
};

/// The `HtlcProofBuilder` can be used to build proofs for transactions
/// that originate in a HTLC contract.
#[derive(Clone, Debug)]
pub struct HtlcProofBuilder {
    pub transaction: Transaction,
    proof: Option<OutgoingHTLCTransactionProof>,
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
    /// use nimiq_transaction_builder::{Recipient, TransactionBuilder, Sender};
    /// use nimiq_hash::{Blake2bHasher, Hasher, HashOutput};
    /// use nimiq_primitives::coin::Coin;
    /// use nimiq_primitives::networks::NetworkId;
    /// use nimiq_primitives::account::AccountType;
    /// # use nimiq_utils::key_rng::SecureGenerate;
    ///
    /// # let key_pair_sender = KeyPair::generate_default_csprng();
    /// # let key_pair_recipient = KeyPair::generate_default_csprng();
    ///
    /// let sender = Sender::new_htlc(Address::from_any_str("NQ46 MNYU LQ93 GYYS P5DC YA51 L5JP UPUT KR62").unwrap());
    /// let recipient = Recipient::new_basic(Address::from(&key_pair_recipient));
    ///
    /// let mut builder = TransactionBuilder::with_required(
    ///     sender,
    ///     recipient,
    ///     Coin::from_u64_unchecked(100),
    ///     1,
    ///     NetworkId::Main
    /// );
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
        SignatureProof::EdDSA(EdDSASignatureProof::from(key_pair.public, signature))
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
    /// use nimiq_transaction_builder::{Recipient, TransactionBuilder, Sender};
    /// use nimiq_hash::{Blake2bHasher, Hasher, HashOutput};
    /// use nimiq_primitives::coin::Coin;
    /// use nimiq_primitives::networks::NetworkId;
    /// use nimiq_primitives::account::AccountType;
    /// # use nimiq_utils::key_rng::SecureGenerate;
    ///
    /// # let key_pair = KeyPair::generate_default_csprng();
    ///
    /// let sender = Sender::new_htlc(Address::from_any_str("NQ46 MNYU LQ93 GYYS P5DC YA51 L5JP UPUT KR62").unwrap());
    /// let recipient = Recipient::new_basic(Address::from(&key_pair));
    ///
    /// let mut builder = TransactionBuilder::with_required(
    ///     sender,
    ///     recipient,
    ///     Coin::from_u64_unchecked(100),
    ///     1,
    ///     NetworkId::Main
    /// );
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
        self.proof = Some(OutgoingHTLCTransactionProof::TimeoutResolve {
            signature_proof_sender: sender_signature,
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
    /// use nimiq_transaction_builder::{Recipient, TransactionBuilder, Sender};
    /// use nimiq_hash::{Blake2bHasher, Hasher, HashOutput};
    /// use nimiq_primitives::coin::Coin;
    /// use nimiq_primitives::networks::NetworkId;
    /// use nimiq_primitives::account::AccountType;
    /// # use nimiq_utils::key_rng::SecureGenerate;
    ///
    /// # let key_pair_sender = KeyPair::generate_default_csprng();
    /// # let key_pair_recipient = KeyPair::generate_default_csprng();
    ///
    /// let sender = Sender::new_htlc(Address::from_any_str("NQ46 MNYU LQ93 GYYS P5DC YA51 L5JP UPUT KR62").unwrap());
    /// let recipient = Recipient::new_basic(Address::from(&key_pair_recipient));
    ///
    /// let mut builder = TransactionBuilder::with_required(
    ///     sender,
    ///     recipient,
    ///     Coin::from_u64_unchecked(100),
    ///     1,
    ///     NetworkId::Main
    /// );
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
        self.proof = Some(OutgoingHTLCTransactionProof::EarlyResolve {
            signature_proof_sender: sender_signature,
            signature_proof_recipient: recipient_signature,
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
        pre_image: PreImage,
        hash_count: u8,
        hash_root: AnyHash,
        recipient_signature: SignatureProof,
    ) -> &mut Self {
        self.proof = Some(OutgoingHTLCTransactionProof::RegularTransfer {
            hash_depth: hash_count,
            hash_root,
            pre_image,
            signature_proof: recipient_signature,
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
    /// use nimiq_transaction_builder::{Recipient, TransactionBuilder, Sender};
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
    /// let sender = Sender::new_htlc(Address::from_any_str("NQ46 MNYU LQ93 GYYS P5DC YA51 L5JP UPUT KR62").unwrap());
    /// let recipient = Recipient::new_basic(Address::from(&key_pair));
    ///
    /// let mut builder = TransactionBuilder::with_required(
    ///     sender,
    ///     recipient,
    ///     Coin::from_u64_unchecked(100),
    ///     1,
    ///     NetworkId::Main
    /// );
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
        self.regular_transfer(
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
    /// use nimiq_transaction_builder::{Recipient, TransactionBuilder, Sender};
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
    /// let sender = Sender::new_htlc(Address::from_any_str("NQ46 MNYU LQ93 GYYS P5DC YA51 L5JP UPUT KR62").unwrap());
    /// let recipient = Recipient::new_basic(Address::from(&key_pair));
    ///
    /// let mut builder = TransactionBuilder::with_required(
    ///     sender,
    ///     recipient,
    ///     Coin::from_u64_unchecked(100),
    ///     1,
    ///     NetworkId::Main
    /// );
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
        self.regular_transfer(
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
