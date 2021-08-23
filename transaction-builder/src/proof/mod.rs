use std::io;

use beserial::Serialize;
use hash::SerializeContent;
use keys::KeyPair;
use primitives::account::AccountType;
use transaction::{SignatureProof, Transaction};

use crate::proof::htlc_contract::HtlcProofBuilder;
use crate::proof::staking_contract::{StakingDataBuilder, StakingProofBuilder};

pub mod htlc_contract;
pub mod staking_contract;

/// The `TransactionProofBuilder` subsumes the builders used to populate a transaction
/// with the required proof to be valid.
/// The proof mostly depends on the sender account (with the exception of incoming staking transactions).
///
/// Thus, there exist four different types of proof builders:
/// - [`StakingDataBuilder`] (that build the staking data and return a normal proof builder)
/// - [`BasicProofBuilder`] (for basic and vesting sender accounts)
/// - [`HtlcProofBuilder`] (for HTLC sender accounts)
/// - [`StakingProofBuilder`] (for outgoing staking transactions)
///
/// [`SignallingProofBuilder`]: staking_contract/struct.StakingDataBuilder.html
/// [`BasicProofBuilder`]: struct.BasicProofBuilder.html
/// [`HtlcProofBuilder`]: htlc_contract/struct.HtlcProofBuilder.html
/// [`StakingProofBuilder`]: staking_contract/struct.StakingProofBuilder.html
#[derive(Clone, Debug)]
pub enum TransactionProofBuilder {
    Basic(BasicProofBuilder),
    Vesting(BasicProofBuilder),
    Htlc(HtlcProofBuilder),
    OutStaking(StakingProofBuilder),
    InStaking(StakingDataBuilder),
}

impl TransactionProofBuilder {
    /// Internal method that ignores incoming staking transactions.
    fn without_in_staking(transaction: Transaction) -> Self {
        match transaction.sender_type {
            AccountType::Basic => {
                TransactionProofBuilder::Basic(BasicProofBuilder::new(transaction))
            }
            AccountType::Vesting => {
                TransactionProofBuilder::Vesting(BasicProofBuilder::new(transaction))
            }
            AccountType::HTLC => TransactionProofBuilder::Htlc(HtlcProofBuilder::new(transaction)),
            AccountType::Staking => {
                TransactionProofBuilder::OutStaking(StakingProofBuilder::new(transaction))
            }
            _ => {
                unreachable!()
            }
        }
    }

    /// Given a `transaction`, this method creates the corresponding proof builder
    /// used to populate it with the required proof.
    pub fn new(transaction: Transaction) -> Self {
        if transaction.recipient_type == AccountType::Staking {
            return TransactionProofBuilder::InStaking(StakingDataBuilder::new(transaction));
        }

        TransactionProofBuilder::without_in_staking(transaction)
    }

    /// This method returns a reference to the preliminary transaction without the required
    /// proof being filled in.
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
    pub fn preliminary_transaction(&self) -> &Transaction {
        match self {
            TransactionProofBuilder::Basic(builder) => &builder.transaction,
            TransactionProofBuilder::Vesting(builder) => &builder.transaction,
            TransactionProofBuilder::Htlc(builder) => &builder.transaction,
            TransactionProofBuilder::OutStaking(builder) => &builder.transaction,
            TransactionProofBuilder::InStaking(builder) => &builder.transaction,
        }
    }

    /// This method can be used for non-signalling transactions where the sender is
    /// a basic account or a vesting contract.
    /// It immediately returns the underlying [`BasicProofBuilder`].
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
    /// let basic_proof_builder = proof_builder.unwrap_basic();
    /// ```
    ///
    /// [`BasicProofBuilder`]: struct.BasicProofBuilder.html
    pub fn unwrap_basic(self) -> BasicProofBuilder {
        match self {
            TransactionProofBuilder::Basic(builder) => builder,
            TransactionProofBuilder::Vesting(builder) => builder,
            _ => panic!("TransactionProofBuilder was not a BasicProofBuilder"),
        }
    }

    /// This method can be used for non-signalling transactions where the sender is a HTLC contract.
    /// It immediately returns the underlying [`HtlcProofBuilder`].
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
    /// [`HtlcProofBuilder`]: htlc_contract/struct.HtlcProofBuilder.html
    pub fn unwrap_htlc(self) -> HtlcProofBuilder {
        match self {
            TransactionProofBuilder::Htlc(builder) => builder,
            _ => panic!("TransactionProofBuilder was not a HtlcProofBuilder"),
        }
    }

    /// This method has to be used for signalling transactions.
    /// It is used to populate the required signalling proof in the data field and can generate
    /// another proof builder for the actual proof field.
    /// This method returns the underlying [`SignallingProofBuilder`].
    ///
    /// # Examples
    ///
    /// ```
    /// use nimiq_keys::{Address, KeyPair};
    /// use nimiq_bls::KeyPair as BlsKeyPair;
    /// use nimiq_transaction_builder::{Recipient, TransactionBuilder};
    /// use nimiq_primitives::coin::Coin;
    /// use nimiq_primitives::networks::NetworkId;
    /// # use nimiq_utils::key_rng::SecureGenerate;
    ///
    /// # let cold_key_pair = KeyPair::generate_default_csprng();
    /// # let warm_key_pair = KeyPair::generate_default_csprng();
    /// # let bls_key_pair = BlsKeyPair::generate_default_csprng();
    /// # let validator_address = Address::from(&cold_key_pair.public);
    ///
    /// let sender_address = Address::from(&cold_key_pair.public);
    /// let mut recipient = Recipient::new_staking_builder();
    /// recipient.update_validator(Some(Address::from(&warm_key_pair)), Some(&bls_key_pair), None, None);
    ///
    /// let tx_builder = TransactionBuilder::with_required(
    ///     sender_address,
    ///     recipient.generate().unwrap(),
    ///     Coin::from_u64_unchecked(0), // must be zero because of signalling transaction
    ///     1,
    ///     NetworkId::Main
    /// );
    ///
    /// let proof_builder = tx_builder.generate().unwrap();
    /// // Unwrap in staking proof builder first.
    /// let mut signalling_proof_builder = proof_builder.unwrap_in_staking();
    /// signalling_proof_builder.sign_with_key_pair(&cold_key_pair);
    ///
    /// let proof_builder = signalling_proof_builder.generate().unwrap();
    /// // Unwrap basic proof builder now.
    /// let mut basic_proof_builder = proof_builder.unwrap_basic();
    /// basic_proof_builder.sign_with_key_pair(&cold_key_pair);
    ///
    /// let final_transaction = basic_proof_builder.generate();
    /// assert!(final_transaction.is_some());
    /// assert!(final_transaction.unwrap().verify(NetworkId::Main).is_ok());
    /// ```
    ///
    /// [`SignallingProofBuilder`]: staking_contract/struct.SignallingProofBuilder.html
    pub fn unwrap_in_staking(self) -> StakingDataBuilder {
        match self {
            TransactionProofBuilder::InStaking(builder) => builder,
            _ => panic!("TransactionProofBuilder was not a StakingDataBuilder"),
        }
    }

    /// This kind of proof builder is used for transactions that move
    /// funds out of the staking contract.
    /// The method returns the underlying [`StakingProofBuilder`].
    ///
    /// # Examples
    ///
    /// ```
    /// use nimiq_keys::{Address, KeyPair};
    /// use nimiq_bls::KeyPair as BlsKeyPair;
    /// use nimiq_transaction_builder::{Recipient, TransactionBuilder};
    /// use nimiq_primitives::coin::Coin;
    /// use nimiq_primitives::networks::NetworkId;
    /// use nimiq_primitives::account::{AccountType};
    /// # use nimiq_utils::key_rng::SecureGenerate;
    /// use nimiq_primitives::policy::STAKING_CONTRACT_ADDRESS;
    ///
    /// # let key_pair = KeyPair::generate_default_csprng();
    /// # let recipient_address = Address::from(&key_pair.public);
    ///
    /// let recipient = Recipient::new_basic(recipient_address);
    /// let staking_contract_address = Address::from_any_str(STAKING_CONTRACT_ADDRESS).unwrap();
    ///
    /// let mut tx_builder = TransactionBuilder::with_required(
    ///     staking_contract_address,
    ///     recipient,
    ///     Coin::from_u64_unchecked(100),
    ///     1,
    ///     NetworkId::Main
    /// );
    /// tx_builder.with_sender_type(AccountType::Staking);
    ///
    /// let proof_builder = tx_builder.generate().unwrap();
    /// // Unwrap staking proof builder.
    /// let mut staking_proof_builder = proof_builder.unwrap_out_staking();
    /// staking_proof_builder.drop_validator(&key_pair);
    ///
    /// let final_transaction = staking_proof_builder.generate();
    /// assert!(final_transaction.is_some());
    /// ```
    ///
    /// [`StakingProofBuilder`]: staking_contract/struct.StakingProofBuilder.html
    pub fn unwrap_out_staking(self) -> StakingProofBuilder {
        match self {
            TransactionProofBuilder::OutStaking(builder) => builder,
            _ => panic!("TransactionProofBuilder was not a StakingProofBuilder"),
        }
    }
}

impl SerializeContent for TransactionProofBuilder {
    fn serialize_content<W: io::Write>(&self, writer: &mut W) -> Result<usize, io::Error> {
        match self {
            TransactionProofBuilder::Basic(builder) => {
                SerializeContent::serialize_content(&builder.transaction, writer)
            }
            TransactionProofBuilder::Vesting(builder) => {
                SerializeContent::serialize_content(&builder.transaction, writer)
            }
            TransactionProofBuilder::Htlc(builder) => {
                SerializeContent::serialize_content(&builder.transaction, writer)
            }
            TransactionProofBuilder::InStaking(builder) => {
                SerializeContent::serialize_content(&builder.transaction, writer)
            }
            TransactionProofBuilder::OutStaking(builder) => {
                SerializeContent::serialize_content(&builder.transaction, writer)
            }
        }
    }
}

/// The `BasicProofBuilder` can be used to build proofs for transactions
/// that originate in basic or vesting accounts, as well as for staking self transactions
/// (i.e., retire/re-activate stake).
#[derive(Clone, Debug)]
pub struct BasicProofBuilder {
    pub transaction: Transaction,
    signature: Option<SignatureProof>,
}

impl BasicProofBuilder {
    /// Creates a new `BasicProofBuilder` from a `transaction`.
    pub fn new(transaction: Transaction) -> Self {
        BasicProofBuilder {
            transaction,
            signature: None,
        }
    }

    /// Manually sets the required `signature` proof for the builder.
    /// In most cases, it is not necessary to call this method.
    /// Instead, it is recommended to automatically generate the signature using [`sign_with_key_pair`].
    ///
    /// [`sign_with_key_pair`]: struct.BasicProofBuilder.html#method.sign_with_key_pair
    pub fn with_signature_proof(&mut self, signature: SignatureProof) -> &mut Self {
        self.signature = Some(signature);
        self
    }

    /// This method sets the required `signature` proof by signing the transaction
    /// using a key pair `key_pair`.
    pub fn sign_with_key_pair(&mut self, key_pair: &KeyPair) -> &mut Self {
        let signature = key_pair.sign(self.transaction.serialize_content().as_slice());
        self.signature = Some(SignatureProof::from(key_pair.public, signature));
        self
    }

    /// This method generates the final transaction if the signature has been set correctly.
    /// Otherwise, it returns `None`.
    pub fn generate(self) -> Option<Transaction> {
        let mut tx = self.transaction;
        tx.proof = self.signature?.serialize_to_vec();
        Some(tx)
    }
}
