use nimiq_bls::{CompressedSignature, KeyPair as BlsKeyPair};
use nimiq_hash::Blake2bHash;
use nimiq_keys::{Address, Ed25519PublicKey as SchnorrPublicKey};
use nimiq_primitives::coin::Coin;
use nimiq_transaction::account::staking_contract::IncomingStakingTransactionData;

use crate::recipient::Recipient;

/// A `StakingRecipientBuilder` can be used to build most transactions interacting
/// with the staking contract (except such transactions that move funds out of the contract).
///
/// Transactions that have the staking contract as a recipient:
///     - Validator
///         * Create
///         * Update (signaling)
///         * Deactivate (signaling)
///         * Reactivate (signaling)
///     - Staker
///         * Create
///         * Stake
///         * Update (signaling)
///
/// Signaling transactions have a special status as they require a zero value
/// as well as an additional step during the proof generation.
/// Also see [`StakingDataBuilder`].
///
/// [`StakingDataBuilder`]: ../../proof/staking_contract/struct.StakingDataBuilder.html
#[derive(Clone, Debug, Default)]
pub struct StakingRecipientBuilder {
    data: Option<IncomingStakingTransactionData>,
}

impl StakingRecipientBuilder {
    /// Creates a new `StakingRecipientBuilder`.
    pub fn new() -> Self {
        Self {
            data: Default::default(),
        }
    }

    /// This method allows to create a new validator entry using two addresses and a BLS key pair.
    /// All rewards for this validator will be paid out to its `reward_address`.
    /// The proof needs to be signed by the cold keypair, which is the key pair that determines the
    /// validator address, and is not an input to this function.
    pub fn create_validator(
        &mut self,
        signing_key: SchnorrPublicKey,
        voting_key_pair: &BlsKeyPair,
        reward_address: Address,
        signal_data: Option<Blake2bHash>,
    ) -> &mut Self {
        self.data = Some(IncomingStakingTransactionData::CreateValidator {
            signing_key,
            voting_key: voting_key_pair.public_key.compress(),
            proof_of_knowledge: StakingRecipientBuilder::generate_proof_of_knowledge(
                voting_key_pair,
            ),
            reward_address,
            signal_data,
            proof: Default::default(),
        });
        self
    }

    /// This method allows to update the details of an existing validator entry. It needs to be
    /// signed by the key pair corresponding to the validator address.
    pub fn update_validator(
        &mut self,
        new_signing_key: Option<SchnorrPublicKey>,
        new_key_pair: Option<&BlsKeyPair>,
        new_reward_address: Option<Address>,
        new_signal_data: Option<Option<Blake2bHash>>,
    ) -> &mut Self {
        self.data = Some(IncomingStakingTransactionData::UpdateValidator {
            new_signing_key,
            new_voting_key: new_key_pair.map(|key| key.public_key.compress()),
            new_proof_of_knowledge: new_key_pair
                .map(StakingRecipientBuilder::generate_proof_of_knowledge),
            new_reward_address,
            new_signal_data,
            proof: Default::default(),
        });
        self
    }

    /// This method allows to deactivate a validator entry. Inactive validators will not be considered
    /// for the validator selection.
    /// It needs to be signed by the key pair corresponding to the signing key.
    pub fn deactivate_validator(&mut self, validator_address: Address) -> &mut Self {
        self.data = Some(IncomingStakingTransactionData::DeactivateValidator {
            validator_address,
            proof: Default::default(),
        });
        self
    }

    /// This method allows to reactivate a validator. This reverts the inactivation of a validator
    /// and will result in the validator being considered for the validator selection again.
    /// This also resets any disabled slots of the given validator if it is selected in current
    /// epoch and has previously been penalized.
    /// It needs to be signed by the key pair corresponding to the signing key.
    pub fn reactivate_validator(&mut self, validator_address: Address) -> &mut Self {
        self.data = Some(IncomingStakingTransactionData::ReactivateValidator {
            validator_address,
            proof: Default::default(),
        });
        self
    }

    /// This method allows to retire a validator entry. Retiring a validator permanently deactivates
    /// it which is necessary in order to delete the validator and retrieve back its initial deposit.
    /// It needs to be signed by the key pair corresponding to the cold key.
    pub fn retire_validator(&mut self) -> &mut Self {
        self.data = Some(IncomingStakingTransactionData::RetireValidator {
            proof: Default::default(),
        });
        self
    }

    /// This method allows to create a staker with a given (optional) delegation to a validator.
    /// It needs to be signed by the key pair corresponding to the staker address.
    pub fn create_staker(&mut self, delegation: Option<Address>) -> &mut Self {
        self.data = Some(IncomingStakingTransactionData::CreateStaker {
            delegation,
            proof: Default::default(),
        });
        self
    }

    /// This method allows to add to a staker's balance.
    pub fn stake(&mut self, staker_address: Address) -> &mut Self {
        self.data = Some(IncomingStakingTransactionData::AddStake { staker_address });
        self
    }

    /// This method allows to change the delegation of a staker.
    /// It needs to be signed by the key pair corresponding to the staker address.
    pub fn update_staker(
        &mut self,
        new_delegation: Option<Address>,
        reactivate_all_stake: bool,
    ) -> &mut Self {
        self.data = Some(IncomingStakingTransactionData::UpdateStaker {
            new_delegation,
            reactivate_all_stake,
            proof: Default::default(),
        });
        self
    }

    /// This method allows to change the active stake balance of the staker and
    /// consequentially its inactive balance. Changing the amount of inactive stake
    /// resets the release time.
    /// It needs to be signed by the key pair corresponding to the staker address.
    pub fn set_active_stake(&mut self, new_active_balance: Coin) -> &mut Self {
        self.data = Some(IncomingStakingTransactionData::SetActiveStake {
            new_active_balance,
            proof: Default::default(),
        });
        self
    }

    /// This method allows to retire the staker from the staker's inactive balance.
    /// Retiring funds is only allowed after the inactive funds are released (i.e. lock-up and jail
    /// periods have passed). Additionally, the minimum total stake for non-retired funds must still
    /// be respected.
    /// It needs to be signed by the key pair corresponding to the staker address.
    pub fn retire_stake(&mut self, retire_stake: Coin) -> &mut Self {
        self.data = Some(IncomingStakingTransactionData::RetireStake {
            retire_stake,
            proof: Default::default(),
        });
        self
    }

    /// A method to generate a proof of knowledge of the secret key by signing the public key.
    pub fn generate_proof_of_knowledge(key_pair: &BlsKeyPair) -> CompressedSignature {
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
    /// use nimiq_keys::{Address, KeyPair};
    /// use nimiq_bls::KeyPair as BlsKeyPair;
    /// use nimiq_utils::key_rng::SecureGenerate;
    ///
    /// let signing_key_pair = KeyPair::generate_default_csprng();
    /// let voting_key_pair = BlsKeyPair::generate_default_csprng();
    /// let reward_address = Address::from_any_str("NQ46 MNYU LQ93 GYYS P5DC YA51 L5JP UPUT KR62").unwrap();
    ///
    /// let mut recipient_builder = Recipient::new_staking_builder();
    /// recipient_builder.create_validator(signing_key_pair.public, &voting_key_pair, reward_address, None);
    /// let recipient = recipient_builder.generate();
    /// assert!(recipient.is_some());
    /// ```
    ///
    /// [`Recipient`]: ../enum.Recipient.html
    pub fn generate(self) -> Option<Recipient> {
        Some(Recipient::Staking { data: self.data? })
    }
}
