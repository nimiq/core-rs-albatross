use bls::{CompressedSignature, KeyPair as BLSKeyPair};
use hash::Blake2bHash;
use keys::Address;
use primitives::coin::Coin;
use transaction::account::staking_contract::IncomingStakingTransactionData;

use crate::recipient::Recipient;

/// A `StakingRecipientBuilder` can be used to build most transactions interacting
/// with the staking contract (except such transactions that move funds out of the contract).
///
/// Transactions that have the staking contract as a recipient:
///     - Validator
///         * Create
///         * Update (signalling)
///         * Retire (signalling)
///         * Re-activate (signalling)
///         * Unpark (signalling)
///     - Staker
///         * Create
///         * Stake
///         * Update (signalling)
///         * Retire (signalling)
///         * Re-activate (signalling)
///
/// Signalling transactions have a special status as they require a zero value
/// as well as an additional step during the proof generation.
/// Also see [`SignallingProofBuilder`].
///
/// [`SignallingProofBuilder`]: ../../proof/staking_contract/struct.SignallingProofBuilder.html
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
        warm_address: Address,
        bls_key_pair: &BLSKeyPair,
        reward_address: Address,
        signal_data: Option<Blake2bHash>,
    ) -> &mut Self {
        self.data = Some(IncomingStakingTransactionData::CreateValidator {
            warm_key: warm_address,
            validator_key: bls_key_pair.public_key.compress(),
            proof_of_knowledge: StakingRecipientBuilder::generate_proof_of_knowledge(bls_key_pair),
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
        new_warm_address: Option<Address>,
        new_key_pair: Option<&BLSKeyPair>,
        new_reward_address: Option<Address>,
        new_signal_data: Option<Option<Blake2bHash>>,
    ) -> &mut Self {
        self.data = Some(IncomingStakingTransactionData::UpdateValidator {
            new_warm_key: new_warm_address,
            new_validator_key: new_key_pair.map(|key| key.public_key.compress()),
            new_proof_of_knowledge: new_key_pair
                .map(|key| StakingRecipientBuilder::generate_proof_of_knowledge(key)),
            new_reward_address,
            new_signal_data,
            proof: Default::default(),
        });
        self
    }

    /// This method allows to retire a validator entry. Inactive validators will not be considered
    /// for the validator selection. Retiring a validator is also necessary to drop it and retrieve
    /// back its initial stake.
    /// It needs to be signed by the key pair corresponding to the warm address.
    pub fn retire_validator(&mut self, validator_address: Address) -> &mut Self {
        self.data = Some(IncomingStakingTransactionData::RetireValidator {
            validator_address,
            proof: Default::default(),
        });
        self
    }

    /// This method allows to re-activate a validator. This reverts the retirement of a validator
    /// and will result in the validator being considered for the validator selection again.
    /// It needs to be signed by the key pair corresponding to the warm address.
    pub fn reactivate_validator(&mut self, validator_address: Address) -> &mut Self {
        self.data = Some(IncomingStakingTransactionData::ReactivateValidator {
            validator_address,
            proof: Default::default(),
        });
        self
    }

    /// This method allows to prevent a validator from being automatically retired after slashing.
    /// After misbehavior or being offline, a validator might be slashed.
    /// This automatically moves a validator into a *parked* state. This means that
    /// this validator will be automatically retired on the next election block.
    /// This signalling transaction will prevent the automatic retirement.
    /// It needs to be signed by the key pair corresponding to the warm address.
    pub fn unpark_validator(&mut self, validator_address: Address) -> &mut Self {
        self.data = Some(IncomingStakingTransactionData::UnparkValidator {
            validator_address,
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

    /// This method allows to add to a staker's active balance.
    pub fn stake(&mut self, staker_address: Address) -> &mut Self {
        self.data = Some(IncomingStakingTransactionData::Stake { staker_address });
        self
    }

    /// This method allows to change the delegation of a staker.
    /// It needs to be signed by the key pair corresponding to the staker address.
    pub fn update_staker(&mut self, new_delegation: Option<Address>) -> &mut Self {
        self.data = Some(IncomingStakingTransactionData::UpdateStaker {
            new_delegation,
            proof: Default::default(),
        });
        self
    }

    /// This method allows to inactivate part, or all, of a staker's active balance.
    /// This has the effect of removing the funds from the validator that the staker is delegating to.
    /// It is a necessary precondition to unstake funds.
    /// It needs to be signed by the key pair corresponding to the staker address.
    pub fn retire_stake(&mut self, value: Coin) -> &mut Self {
        self.data = Some(IncomingStakingTransactionData::RetireStaker {
            value,
            proof: Default::default(),
        });
        self
    }

    /// This method allows to reactivate part, or all, of a staker's inactive balance.
    /// This has the effect of adding the funds to the validator that the staker is delegating to.
    /// It needs to be signed by the key pair corresponding to the staker address.
    pub fn reactivate_stake(&mut self, value: Coin) -> &mut Self {
        self.data = Some(IncomingStakingTransactionData::ReactivateStaker {
            value,
            proof: Default::default(),
        });
        self
    }

    /// A method to generate a proof of knowledge of the secret key by signing the public key.
    pub fn generate_proof_of_knowledge(key_pair: &BLSKeyPair) -> CompressedSignature {
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
    /// use nimiq_bls::KeyPair as BLSKeyPair;
    /// use nimiq_utils::key_rng::SecureGenerate;
    ///
    /// let warm_key_pair = KeyPair::generate_default_csprng();
    /// let warm_address = Address::from(&warm_key_pair);
    /// let validator_key_pair = BLSKeyPair::generate_default_csprng();
    /// let reward_address = Address::from_any_str("NQ46 MNYU LQ93 GYYS P5DC YA51 L5JP UPUT KR62").unwrap();
    ///
    /// let mut recipient_builder = Recipient::new_staking_builder();
    /// recipient_builder.create_validator(warm_address, &validator_key_pair, reward_address, None);
    /// let recipient = recipient_builder.generate();
    /// assert!(recipient.is_some());
    /// ```
    ///
    /// [`Recipient`]: ../enum.Recipient.html
    pub fn generate(self) -> Option<Recipient> {
        Some(Recipient::Staking { data: self.data? })
    }
}
