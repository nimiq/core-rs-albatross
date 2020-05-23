use beserial::{Serialize, SerializingError, WriteBytesExt};
use bls::{CompressedSignature, KeyPair, PublicKey};
use keys::Address;
use transaction::account::staking_contract::{
    IncomingStakingTransactionData, SelfStakingTransactionData,
};
use utils::key_rng::SecureGenerate;

use crate::recipient::Recipient;

/// This datatype represents the two types of transactions that can have a staking contract
/// as recipient:
/// - Incoming transactions (such as staking or signalling transactions)
/// - Self transactions (to retire/re-activate a stake)
pub enum StakingTransaction {
    IncomingTransaction(IncomingStakingTransactionData),
    SelfTransaction(SelfStakingTransactionData),
}

impl StakingTransaction {
    /// This method determines whether the transaction is a signalling transaction
    /// (i.e., a zero value transaction).
    pub fn is_signalling(&self) -> bool {
        match self {
            StakingTransaction::IncomingTransaction(data) => data.is_signalling(),
            _ => false,
        }
    }

    /// This method determines whether the transaction is a self transaction
    /// (i.e., both sender and recipient are the staking contract).
    pub fn is_self_transaction(&self) -> bool {
        match self {
            StakingTransaction::SelfTransaction(_) => true,
            _ => false,
        }
    }
}

impl Serialize for StakingTransaction {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        match self {
            StakingTransaction::IncomingTransaction(data) => data.serialize(writer),
            StakingTransaction::SelfTransaction(data) => data.serialize(writer),
        }
    }

    fn serialized_size(&self) -> usize {
        match self {
            StakingTransaction::IncomingTransaction(data) => data.serialized_size(),
            StakingTransaction::SelfTransaction(data) => data.serialized_size(),
        }
    }
}

/// A `StakingRecipientBuilder` can be used to build most transactions interacting
/// with the staking contract (except such transactions that move funds out of the contract).
///
/// We need to distinguish two types of transactions that have the staking contract as a recipient:
/// 1. Incoming transactions, which include:
///     - Validator
///         * Create
///         * Update (signalling)
///         * Retire (signalling)
///         * Re-activate (signalling)
///         * Unpark (signalling)
///     - Staker
///         * Stake
/// 2. Self transactions, which include:
///     - Staker
///         * Retire
///         * Re-activate
///
/// Signalling transactions have a special status as they require a zero value
/// as well as an additional step during the proof generation.
/// Also see [`SignallingProofBuilder`].
///
/// [`SignallingProofBuilder`]: ../../proof/staking_contract/struct.SignallingProofBuilder.html
pub struct StakingRecipientBuilder {
    staking_contract: Address,
    staking_data: Option<StakingTransaction>,
}

impl StakingRecipientBuilder {
    /// Creates a new `StakingRecipientBuilder` for the staking contract
    /// at address `staking_contract`.
    pub fn new(staking_contract: Address) -> Self {
        StakingRecipientBuilder {
            staking_contract,
            staking_data: Default::default(),
        }
    }

    /// This method allows to create a new validator entry using a BLS key pair `key_pair`.
    /// All rewards for this validator will be paid out to its `reward_address`.
    pub fn create_validator(&mut self, key_pair: &KeyPair, reward_address: Address) -> &mut Self {
        self.staking_data = Some(StakingTransaction::IncomingTransaction(
            IncomingStakingTransactionData::CreateValidator {
                validator_key: key_pair.public_key.compress(),
                proof_of_knowledge: StakingRecipientBuilder::generate_proof_of_knowledge(&key_pair),
                reward_address,
            },
        ));
        self
    }

    /// This method allows to create a new validator entry and also generates a new key pair for it.
    /// All rewards for this validator will be paid out to its `reward_address`.
    ///
    /// The method returns the generated BLS key pair.
    pub fn create_validator_with_new_bls_key(&mut self, reward_address: Address) -> KeyPair {
        let key = KeyPair::generate_default_csprng();
        self.create_validator(&key, reward_address);
        key
    }

    /// This method allows to update the details of an existing validator entry with the
    /// public key `old_validator_key`.
    /// Both the key pair and the reward address can be updated.
    /// All updates will only take effect starting in the following epoch.
    pub fn update_validator(
        &mut self,
        old_validator_key: &PublicKey,
        new_key_pair: Option<&KeyPair>,
        new_reward_address: Option<Address>,
    ) -> &mut Self {
        self.staking_data = Some(StakingTransaction::IncomingTransaction(
            IncomingStakingTransactionData::UpdateValidator {
                old_validator_key: old_validator_key.compress(),
                new_validator_key: new_key_pair.map(|key| key.public_key.compress()),
                new_proof_of_knowledge: new_key_pair
                    .map(|key| StakingRecipientBuilder::generate_proof_of_knowledge(&key)),
                new_reward_address,
                signature: Default::default(),
            },
        ));
        self
    }

    /// This method allows to retire a validator entry.
    /// Inactive validators will not be considered for the validator selection.
    ///
    /// Retiring a validator is also necessary to drop it and retrieve back its initial stake.
    pub fn retire_validator(&mut self, validator_key: &PublicKey) -> &mut Self {
        self.staking_data = Some(StakingTransaction::IncomingTransaction(
            IncomingStakingTransactionData::RetireValidator {
                validator_key: validator_key.compress(),
                signature: Default::default(),
            },
        ));
        self
    }

    /// This method allows to re-activate a validator.
    /// This reverts the retirement of a validator and will result in the validator being
    /// considered for the validator selection again.
    pub fn reactivate_validator(&mut self, validator_key: &PublicKey) -> &mut Self {
        self.staking_data = Some(StakingTransaction::IncomingTransaction(
            IncomingStakingTransactionData::ReactivateValidator {
                validator_key: validator_key.compress(),
                signature: Default::default(),
            },
        ));
        self
    }

    /// This method allows to prevent a validator from being automatically retired after slashing.
    ///
    /// After misbehavior or being offline, a validator might be slashed.
    /// This automatically moves a validator into a *parking* state. This means that
    /// this validator is still considered for the next validator selection, but will be
    /// automatically retired after two macro blocks.
    /// This signalling transaction will prevent the automatic retirement.
    pub fn unpark_validator(&mut self, validator_key: &PublicKey) -> &mut Self {
        self.staking_data = Some(StakingTransaction::IncomingTransaction(
            IncomingStakingTransactionData::UnparkValidator {
                validator_key: validator_key.compress(),
                signature: Default::default(),
            },
        ));
        self
    }

    /// This method allows to delegate stake to a validator with public key `validator_key`.
    /// Optionally, the stake can be delegated in the name of an address different than
    /// the sender of the transaction by using the `staker_address`.
    pub fn stake(
        &mut self,
        validator_key: &PublicKey,
        staker_address: Option<Address>,
    ) -> &mut Self {
        self.staking_data = Some(StakingTransaction::IncomingTransaction(
            IncomingStakingTransactionData::Stake {
                validator_key: validator_key.compress(),
                staker_address,
            },
        ));
        self
    }

    /// This method allows to retire a stake.
    /// This has the effect of removing the stake from the validator with key `validator_key`.
    /// It is a necessary precondition to unstake funds.
    pub fn retire_stake(&mut self, validator_key: &PublicKey) -> &mut Self {
        self.staking_data = Some(StakingTransaction::SelfTransaction(
            SelfStakingTransactionData::RetireStake(validator_key.compress()),
        ));
        self
    }

    /// This method allows to reassign inactive stake to a validator with key `validator_key`.
    pub fn reactivate_stake(&mut self, validator_key: &PublicKey) -> &mut Self {
        self.staking_data = Some(StakingTransaction::SelfTransaction(
            SelfStakingTransactionData::ReactivateStake(validator_key.compress()),
        ));
        self
    }

    /// A method to generate a proof of knowledge of the secret key by signing the public key.
    pub fn generate_proof_of_knowledge(key_pair: &KeyPair) -> CompressedSignature {
        key_pair.sign(&key_pair.public_key).compress()
    }

    /// This method tries putting together the staking transaction,
    /// returning a [`Recipient`] in case of success.
    /// In case of a failure, it returns `None`.
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
    /// let reward_address = Address::from_any_str("NQ46 MNYU LQ93 GYYS P5DC YA51 L5JP UPUT KR62").unwrap();
    /// let mut recipient_builder = Recipient::new_staking_builder(staking_contract);
    /// recipient_builder.create_validator(&validator_key_pair, reward_address);
    /// let recipient = recipient_builder.generate();
    /// assert!(recipient.is_some());
    /// ```
    ///
    /// [`Recipient`]: ../enum.Recipient.html
    pub fn generate(self) -> Option<Recipient> {
        Some(Recipient::Staking {
            address: self.staking_contract,
            data: self.staking_data?,
        })
    }
}
