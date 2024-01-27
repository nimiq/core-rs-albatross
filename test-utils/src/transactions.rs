use log::debug;
use nimiq_account::{
    Account, Accounts, BasicAccount, BlockState, HashedTimeLockedContract,
    StakingContractStoreWrite, TransactionLog, VestingContract,
};
use nimiq_bls::KeyPair as BlsKeyPair;
use nimiq_database::traits::{Database, WriteTransaction};
use nimiq_hash::{Blake2bHasher, Hasher};
use nimiq_keys::{Address, KeyPair, SecureGenerate};
use nimiq_primitives::{
    account::AccountType, coin::Coin, key_nibbles::KeyNibbles, networks::NetworkId, policy::Policy,
};
use nimiq_serde::Serialize;
use nimiq_transaction::{
    account::{
        htlc_contract::{
            AnyHash, AnyHash32, CreationTransactionData as HTLCCreationTransactionData, PreImage,
        },
        staking_contract::{IncomingStakingTransactionData, OutgoingStakingTransactionData},
        vesting_contract::CreationTransactionData as VestingCreationTransactionData,
    },
    SignatureProof, Transaction,
};
use nimiq_transaction_builder::TransactionProofBuilder;
use nimiq_trie::WriteTransactionProxy;
use rand::{CryptoRng, Rng};

pub enum ValidatorState {
    Active,
    Inactive,
    Retired,
}

#[derive(Clone, Copy, Debug)]
pub enum OutgoingType {
    Basic,
    Vesting,
    HTLCRegularTransfer,
    HTLCEarlyResolve,
    HTLCTimeoutResolve,
    // Staking Contract types
    DeleteValidator,
    RemoveStake,
}

impl From<OutgoingType> for AccountType {
    fn from(value: OutgoingType) -> Self {
        match value {
            OutgoingType::Basic => AccountType::Basic,
            OutgoingType::Vesting => AccountType::Vesting,
            OutgoingType::HTLCRegularTransfer => AccountType::HTLC,
            OutgoingType::HTLCEarlyResolve => AccountType::HTLC,
            OutgoingType::HTLCTimeoutResolve => AccountType::HTLC,
            OutgoingType::DeleteValidator => AccountType::Staking,
            OutgoingType::RemoveStake => AccountType::Staking,
        }
    }
}

impl OutgoingType {
    pub fn is_staking(&self) -> bool {
        matches!(
            self,
            OutgoingType::RemoveStake | OutgoingType::DeleteValidator
        )
    }
}

#[derive(Clone, Copy, Debug)]
pub enum IncomingType {
    Basic,
    CreateVesting,
    CreateHTLC,
    // Staking Contract types
    CreateValidator,
    UpdateValidator,
    DeactivateValidator,
    ReactivateValidator,
    RetireValidator,
    CreateStaker,
    AddStake,
    UpdateStaker,
    SetActiveStake,
    RetireStake,
}

impl IncomingType {
    pub fn is_validator_related(&self) -> bool {
        matches!(
            self,
            IncomingType::CreateValidator
                | IncomingType::UpdateValidator
                | IncomingType::DeactivateValidator
                | IncomingType::ReactivateValidator
                | IncomingType::RetireValidator
        )
    }

    pub fn is_staker_related(&self) -> bool {
        matches!(
            self,
            IncomingType::CreateStaker
                | IncomingType::AddStake
                | IncomingType::UpdateStaker
                | IncomingType::SetActiveStake
                | IncomingType::RetireStake
        )
    }
}

impl From<IncomingType> for AccountType {
    fn from(value: IncomingType) -> Self {
        match value {
            IncomingType::Basic => AccountType::Basic,
            IncomingType::CreateVesting => AccountType::Vesting,
            IncomingType::CreateHTLC => AccountType::HTLC,
            IncomingType::CreateValidator => AccountType::Staking,
            IncomingType::UpdateValidator => AccountType::Staking,
            IncomingType::DeactivateValidator => AccountType::Staking,
            IncomingType::ReactivateValidator => AccountType::Staking,
            IncomingType::RetireValidator => AccountType::Staking,
            IncomingType::CreateStaker => AccountType::Staking,
            IncomingType::AddStake => AccountType::Staking,
            IncomingType::UpdateStaker => AccountType::Staking,
            IncomingType::SetActiveStake => AccountType::Staking,
            IncomingType::RetireStake => AccountType::Staking,
        }
    }
}

enum OutgoingAccountData {
    Basic {
        key_pair: KeyPair,
    },
    Vesting {
        contract_address: Address,
        owner_key_pair: KeyPair,
    },
    Htlc {
        contract_address: Address,
        sender_key_pair: KeyPair,
        recipient_key_pair: KeyPair,
        pre_image: PreImage,
        hash_root: AnyHash,
    },
    Staking {
        validator_key_pair: KeyPair,
        staker_key_pair: KeyPair,
    },
}

impl OutgoingAccountData {
    fn sender_address(&self) -> Address {
        match self {
            OutgoingAccountData::Basic { key_pair } => Address::from(key_pair),
            OutgoingAccountData::Vesting {
                contract_address, ..
            } => contract_address.clone(),
            OutgoingAccountData::Htlc {
                contract_address, ..
            } => contract_address.clone(),
            OutgoingAccountData::Staking { .. } => Policy::STAKING_CONTRACT_ADDRESS.clone(),
        }
    }
}

enum IncomingAccountData {
    Basic {
        address: Address,
    },
    Vesting {
        parameters: VestingCreationTransactionData,
    },
    Htlc {
        parameters: HTLCCreationTransactionData,
    },
    Staking {
        validator_key_pair: KeyPair,
        staker_key_pair: KeyPair,
        parameters: IncomingStakingTransactionData,
    },
}

/// Can only generate valid transactions.
pub struct TransactionsGenerator<R: Rng + CryptoRng> {
    accounts: Accounts,
    network_id: NetworkId,
    rng: R,
}

impl<R: Rng + CryptoRng> TransactionsGenerator<R> {
    pub fn new(accounts: Accounts, network_id: NetworkId, rng: R) -> Self {
        TransactionsGenerator {
            accounts,
            network_id,
            rng,
        }
    }

    pub fn create_failing_transaction(
        &mut self,
        sender: OutgoingType,
        recipient: IncomingType,
        mut value: Coin,
        fee: Coin,
        block_state: &BlockState,
        fail_sender: bool,
        fail_recipient: bool,
    ) -> Option<Transaction> {
        if sender.is_staking()
            && (recipient.is_validator_related() || recipient.is_staker_related())
        {
            return None;
        }

        // Ensure that the value is the validator deposit if tx should succeed. Otherwise, makes value higher
        // than validator deposit.
        match sender {
            OutgoingType::DeleteValidator => {
                value = Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT)
                    .checked_sub(fee)
                    .expect("Fee must not be higher than validator deposit");
                if fail_sender {
                    value += Coin::from_u64_unchecked(1);
                }
            }
            OutgoingType::RemoveStake => {
                value = Coin::from_u64_unchecked(2 * Policy::MINIMUM_STAKE)
                    .checked_sub(fee)
                    .expect("Fee must not be higher than staker stake");
                if fail_sender {
                    value += Coin::from_u64_unchecked(1);
                }
            }
            _ => {}
        }

        // Ensure that the value is the validator deposit if we are creating a validator.
        // Ensure that the minimum stake is respected in staker operations.
        match recipient {
            IncomingType::CreateValidator => {
                value = Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT);
            }
            IncomingType::SetActiveStake => {
                value = Coin::ZERO;
            }
            IncomingType::CreateStaker | IncomingType::RetireStake => {
                value = Coin::from_u64_unchecked(Policy::MINIMUM_STAKE);
            }
            _ => {}
        }

        let sender_data = match sender {
            OutgoingType::RemoveStake => {
                OutgoingStakingTransactionData::RemoveStake.serialize_to_vec()
            }
            OutgoingType::DeleteValidator => {
                OutgoingStakingTransactionData::DeleteValidator.serialize_to_vec()
            }
            _ => {
                vec![]
            }
        };

        let sender_account =
            self.ensure_outgoing_account(sender, value, fee, block_state, fail_sender);
        let recipient_account = self.ensure_incoming_account(recipient, value, fail_recipient);

        // First, create the preliminary unsigned transaction.
        let tx = match recipient_account {
            IncomingAccountData::Basic { ref address } => Transaction::new_extended(
                sender_account.sender_address(),
                sender.into(),
                sender_data,
                address.clone(),
                recipient.into(),
                vec![],
                value,
                fee,
                block_state.number,
                self.network_id,
            ),
            IncomingAccountData::Vesting { ref parameters } => Transaction::new_contract_creation(
                sender_account.sender_address(),
                sender.into(),
                sender_data,
                recipient.into(),
                parameters.serialize_to_vec(),
                value,
                fee,
                block_state.number,
                self.network_id,
            ),
            IncomingAccountData::Htlc { ref parameters } => Transaction::new_contract_creation(
                sender_account.sender_address(),
                sender.into(),
                sender_data,
                recipient.into(),
                parameters.serialize_to_vec(),
                value,
                fee,
                block_state.number,
                self.network_id,
            ),
            IncomingAccountData::Staking { ref parameters, .. } => {
                if parameters.is_signaling() {
                    // We cannot make txs with value = 0 from a basic account fail.
                    if fail_sender {
                        return None;
                    }
                    Transaction::new_signaling(
                        sender_account.sender_address(),
                        sender.into(),
                        Policy::STAKING_CONTRACT_ADDRESS.clone(),
                        recipient.into(),
                        fee,
                        parameters.serialize_to_vec(),
                        block_state.number,
                        self.network_id,
                    )
                } else {
                    Transaction::new_extended(
                        sender_account.sender_address(),
                        sender.into(),
                        sender_data,
                        Policy::STAKING_CONTRACT_ADDRESS.clone(),
                        recipient.into(),
                        parameters.serialize_to_vec(),
                        value,
                        fee,
                        block_state.number,
                        self.network_id,
                    )
                }
            }
        };

        // Second, create a proof builder.
        let mut proof_builder = TransactionProofBuilder::new(tx);

        // Sign signalling transactions.
        if let IncomingAccountData::Staking {
            ref validator_key_pair,
            ref staker_key_pair,
            ..
        } = recipient_account
        {
            let mut in_staking_proof_builder = proof_builder.unwrap_in_staking();
            if recipient.is_validator_related() {
                in_staking_proof_builder.sign_with_key_pair(validator_key_pair);
            } else {
                in_staking_proof_builder.sign_with_key_pair(staker_key_pair);
            }
            proof_builder = in_staking_proof_builder
                .generate()
                .expect("Cannot sign signalling transaction");
        }

        // Populate proof field of transaction.
        Some(match sender_account {
            OutgoingAccountData::Basic { key_pair } => {
                let mut basic_proof_builder = proof_builder.unwrap_basic();
                basic_proof_builder.sign_with_key_pair(&key_pair);
                basic_proof_builder
                    .generate()
                    .expect("Cannot create sender proof")
            }
            OutgoingAccountData::Vesting { owner_key_pair, .. } => {
                let mut basic_proof_builder = proof_builder.unwrap_basic();
                basic_proof_builder.sign_with_key_pair(&owner_key_pair);
                basic_proof_builder
                    .generate()
                    .expect("Cannot create sender proof")
            }
            OutgoingAccountData::Htlc {
                sender_key_pair,
                recipient_key_pair,
                pre_image,
                hash_root,
                ..
            } => {
                let mut htlc_proof_builder = proof_builder.unwrap_htlc();

                match sender {
                    OutgoingType::HTLCRegularTransfer => htlc_proof_builder.regular_transfer(
                        pre_image,
                        1,
                        hash_root,
                        htlc_proof_builder.signature_with_key_pair(&recipient_key_pair),
                    ),
                    OutgoingType::HTLCEarlyResolve => htlc_proof_builder.early_resolve(
                        htlc_proof_builder.signature_with_key_pair(&sender_key_pair),
                        htlc_proof_builder.signature_with_key_pair(&recipient_key_pair),
                    ),
                    OutgoingType::HTLCTimeoutResolve => htlc_proof_builder.timeout_resolve(
                        htlc_proof_builder.signature_with_key_pair(&sender_key_pair),
                    ),
                    _ => unreachable!(),
                };

                htlc_proof_builder
                    .generate()
                    .expect("Cannot create sender proof")
            }
            OutgoingAccountData::Staking {
                validator_key_pair,
                staker_key_pair,
                ..
            } => {
                let mut out_staking_proof_builder = proof_builder.unwrap_out_staking();
                match sender {
                    OutgoingType::DeleteValidator => {
                        out_staking_proof_builder.sign_with_key_pair(&validator_key_pair);
                    }
                    OutgoingType::RemoveStake => {
                        out_staking_proof_builder.sign_with_key_pair(&staker_key_pair);
                    }
                    _ => unreachable!(),
                };

                out_staking_proof_builder
                    .generate()
                    .expect("Cannot create sender proof")
            }
        })
    }

    pub fn put_account(&self, address: &Address, account: Account) {
        let mut raw_txn = self.accounts.env.write_transaction();
        let mut txn: WriteTransactionProxy = (&mut raw_txn).into();
        self.accounts
            .tree
            .put(&mut txn, &KeyNibbles::from(address), account)
            .expect("Failed to put initial accounts");
        self.accounts
            .tree
            .update_root(&mut txn)
            .expect("Tree must be complete");
        raw_txn.commit();
    }

    fn ensure_outgoing_account(
        &mut self,
        outgoing_type: OutgoingType,
        value: Coin,
        fee: Coin,
        block_state: &BlockState,
        fail_sender: bool,
    ) -> OutgoingAccountData {
        // If we intend to generate a failing tx, we create an account only capable of paying the fee.
        let mut balance = value + fee;
        if fail_sender {
            balance = fee;
        }

        match outgoing_type {
            OutgoingType::Basic => {
                // We create a new account with that balance.
                let key_pair = KeyPair::generate(&mut self.rng);
                let account = Account::Basic(BasicAccount { balance });

                self.put_account(&Address::from(&key_pair), account);
                OutgoingAccountData::Basic { key_pair }
            }
            OutgoingType::Vesting => {
                // We create a new account with that balance.
                let owner_key_pair = KeyPair::generate(&mut self.rng);
                // Vesting account releases full balance basically immediately.
                let account = Account::Vesting(VestingContract {
                    balance,
                    owner: Address::from(&owner_key_pair),
                    start_time: 0,
                    step_amount: balance,
                    time_step: 1,
                    total_amount: balance,
                });
                let contract_address = Address(self.rng.gen());

                debug!(?contract_address, "Create vesting contract");
                self.put_account(&contract_address, account);
                OutgoingAccountData::Vesting {
                    contract_address,
                    owner_key_pair,
                }
            }
            OutgoingType::HTLCEarlyResolve
            | OutgoingType::HTLCRegularTransfer
            | OutgoingType::HTLCTimeoutResolve => {
                // We create a new account with that balance.
                let sender_key_pair = KeyPair::generate(&mut self.rng);
                let recipient_key_pair = KeyPair::generate(&mut self.rng);

                let timeout = if matches!(outgoing_type, OutgoingType::HTLCTimeoutResolve) {
                    block_state.time.saturating_sub(1)
                } else {
                    block_state.time.saturating_add(10_000_000)
                };

                // HTLC account.
                let pre_image = PreImage::PreImage32(AnyHash32(self.rng.gen()));
                let hash_root = AnyHash::from(
                    Blake2bHasher::default()
                        .chain(&pre_image.as_bytes())
                        .finish(),
                );
                let account = Account::HTLC(HashedTimeLockedContract {
                    balance,
                    sender: Address::from(&sender_key_pair),
                    recipient: Address::from(&recipient_key_pair),
                    hash_root: hash_root.clone(),
                    hash_count: 1,
                    timeout,
                    total_amount: balance,
                });
                let contract_address = Address(self.rng.gen());

                debug!(?contract_address, "Create HTLC contract");
                self.put_account(&contract_address, account);
                OutgoingAccountData::Htlc {
                    contract_address,
                    sender_key_pair,
                    recipient_key_pair,
                    pre_image,
                    hash_root,
                }
            }
            OutgoingType::DeleteValidator => {
                let (validator_key_pair, _, staker_key_pair) =
                    self.create_validator_and_staker(ValidatorState::Retired, true, false);
                OutgoingAccountData::Staking {
                    validator_key_pair,
                    staker_key_pair,
                }
            }
            OutgoingType::RemoveStake => {
                let (validator_key_pair, _, staker_key_pair) =
                    self.create_validator_and_staker(ValidatorState::Retired, true, true);
                OutgoingAccountData::Staking {
                    validator_key_pair,
                    staker_key_pair,
                }
            }
        }
    }

    fn ensure_incoming_account(
        &mut self,
        incoming_type: IncomingType,
        balance: Coin,
        fail_recipient: bool,
    ) -> IncomingAccountData {
        if fail_recipient
            && matches!(
                incoming_type,
                IncomingType::Basic | IncomingType::CreateVesting | IncomingType::CreateHTLC
            )
        {
            panic!("Cannot fail a {:?} account recipient", incoming_type);
        }

        match incoming_type {
            IncomingType::Basic => IncomingAccountData::Basic {
                address: Address(self.rng.gen()),
            },
            IncomingType::CreateVesting => IncomingAccountData::Vesting {
                parameters: VestingCreationTransactionData {
                    owner: Address(self.rng.gen()),
                    start_time: 0,
                    time_step: 1,
                    step_amount: balance,
                    total_amount: balance,
                },
            },
            IncomingType::CreateHTLC => IncomingAccountData::Htlc {
                parameters: HTLCCreationTransactionData {
                    sender: Address(self.rng.gen()),
                    recipient: Address(self.rng.gen()),
                    hash_root: AnyHash::default(),
                    hash_count: 1,
                    timeout: 100,
                },
            },
            IncomingType::CreateValidator => {
                let (existing_validator_key_pair, _, staker_key_pair) =
                    self.create_validator_and_staker(ValidatorState::Active, false, false);
                let mut validator_key_pair = KeyPair::generate(&mut self.rng);
                let validator_voting_key_pair = BlsKeyPair::generate(&mut self.rng);
                let validator_voting_key_compressed =
                    validator_voting_key_pair.public_key.compress();

                // We can make the transaction fail by using an existing validator address.
                if fail_recipient {
                    validator_key_pair = existing_validator_key_pair;
                }

                IncomingAccountData::Staking {
                    parameters: IncomingStakingTransactionData::CreateValidator {
                        signing_key: validator_key_pair.public,
                        voting_key: validator_voting_key_compressed.clone(),
                        reward_address: Address(self.rng.gen()),
                        signal_data: None,
                        proof_of_knowledge: validator_voting_key_pair
                            .sign(&validator_voting_key_compressed.serialize_to_vec())
                            .compress(),
                        proof: SignatureProof::default(),
                    },
                    validator_key_pair,
                    staker_key_pair,
                }
            }
            IncomingType::UpdateValidator => {
                let (mut validator_key_pair, _, staker_key_pair) =
                    self.create_validator_and_staker(ValidatorState::Active, false, false);

                let new_validator_key_pair = KeyPair::generate(&mut self.rng);
                let new_validator_voting_key_pair = BlsKeyPair::generate(&mut self.rng);
                let new_validator_voting_key_compressed =
                    new_validator_voting_key_pair.public_key.compress();

                // We can make the transaction fail by using a non-existing validator address.
                if fail_recipient {
                    validator_key_pair = KeyPair::generate(&mut self.rng);
                }

                IncomingAccountData::Staking {
                    parameters: IncomingStakingTransactionData::UpdateValidator {
                        new_signing_key: Some(new_validator_key_pair.public),
                        new_voting_key: Some(new_validator_voting_key_compressed.clone()),
                        new_reward_address: Some(Address(self.rng.gen())),
                        new_signal_data: None,
                        new_proof_of_knowledge: Some(
                            new_validator_voting_key_pair
                                .sign(&new_validator_voting_key_compressed.serialize_to_vec())
                                .compress(),
                        ),
                        proof: SignatureProof::default(),
                    },
                    validator_key_pair,
                    staker_key_pair,
                }
            }
            IncomingType::DeactivateValidator => {
                let (mut validator_key_pair, _, staker_key_pair) =
                    self.create_validator_and_staker(ValidatorState::Active, false, false);

                // We can make the transaction fail by using a non-existing validator address.
                if fail_recipient {
                    validator_key_pair = KeyPair::generate(&mut self.rng);
                }

                IncomingAccountData::Staking {
                    parameters: IncomingStakingTransactionData::DeactivateValidator {
                        validator_address: Address::from(&validator_key_pair),
                        proof: SignatureProof::default(),
                    },
                    validator_key_pair,
                    staker_key_pair,
                }
            }
            IncomingType::ReactivateValidator => {
                let (mut validator_key_pair, _, staker_key_pair) =
                    self.create_validator_and_staker(ValidatorState::Inactive, false, false);

                // We can make the transaction fail by using a non-existing validator address.
                if fail_recipient {
                    validator_key_pair = KeyPair::generate(&mut self.rng);
                }

                IncomingAccountData::Staking {
                    parameters: IncomingStakingTransactionData::ReactivateValidator {
                        validator_address: Address::from(&validator_key_pair),
                        proof: SignatureProof::default(),
                    },
                    validator_key_pair,
                    staker_key_pair,
                }
            }
            IncomingType::RetireValidator => {
                let (mut validator_key_pair, _, staker_key_pair) =
                    self.create_validator_and_staker(ValidatorState::Active, false, false);

                // We can make the transaction fail by using a non-existing validator address.
                if fail_recipient {
                    validator_key_pair = KeyPair::generate(&mut self.rng);
                }

                IncomingAccountData::Staking {
                    parameters: IncomingStakingTransactionData::RetireValidator {
                        proof: SignatureProof::default(),
                    },
                    validator_key_pair,
                    staker_key_pair,
                }
            }
            IncomingType::CreateStaker => {
                let (validator_key_pair, _, existing_staker_key_pair) =
                    self.create_validator_and_staker(ValidatorState::Active, false, false);

                let mut staker_key_pair = KeyPair::generate(&mut self.rng);

                // We can make the transaction fail by using an existing staker address.
                if fail_recipient {
                    staker_key_pair = existing_staker_key_pair;
                }

                IncomingAccountData::Staking {
                    parameters: IncomingStakingTransactionData::CreateStaker {
                        delegation: Some(Address::from(&validator_key_pair)),
                        proof: SignatureProof::default(),
                    },
                    validator_key_pair,
                    staker_key_pair,
                }
            }
            IncomingType::AddStake => {
                let (validator_key_pair, _, mut staker_key_pair) =
                    self.create_validator_and_staker(ValidatorState::Active, false, false);

                // We can make the transaction fail by using a non-existing staker address.
                if fail_recipient {
                    staker_key_pair = KeyPair::generate(&mut self.rng);
                }

                IncomingAccountData::Staking {
                    parameters: IncomingStakingTransactionData::AddStake {
                        staker_address: Address::from(&staker_key_pair),
                    },
                    validator_key_pair,
                    staker_key_pair,
                }
            }
            IncomingType::SetActiveStake => {
                let (validator_key_pair, _, mut staker_key_pair) =
                    self.create_validator_and_staker(ValidatorState::Active, false, false);

                // We can make the transaction fail by using a non-existing staker address.
                if fail_recipient {
                    staker_key_pair = KeyPair::generate(&mut self.rng);
                }

                IncomingAccountData::Staking {
                    parameters: IncomingStakingTransactionData::SetActiveStake {
                        new_active_balance: balance,
                        proof: SignatureProof::default(),
                    },
                    validator_key_pair,
                    staker_key_pair,
                }
            }
            IncomingType::UpdateStaker => {
                // Create a staker that stakes for a validator.
                let (_, _, mut staker_key_pair) =
                    self.create_validator_and_staker(ValidatorState::Active, true, false);
                // Then create another validator to switch to.
                let (validator_key_pair, _, _) =
                    self.create_validator_and_staker(ValidatorState::Active, false, false);

                // We can make the transaction fail by using a non-existing staker address.
                if fail_recipient {
                    staker_key_pair = KeyPair::generate(&mut self.rng);
                }

                IncomingAccountData::Staking {
                    parameters: IncomingStakingTransactionData::UpdateStaker {
                        new_delegation: Some(Address::from(&validator_key_pair)),
                        reactivate_all_stake: false,
                        proof: SignatureProof::default(),
                    },
                    validator_key_pair,
                    staker_key_pair,
                }
            }
            IncomingType::RetireStake => {
                let (validator_key_pair, _, mut staker_key_pair) =
                    self.create_validator_and_staker(ValidatorState::Active, false, false);

                // We can make the transaction fail by using a non-existing staker address.
                if fail_recipient {
                    staker_key_pair = KeyPair::generate(&mut self.rng);
                }

                IncomingAccountData::Staking {
                    parameters: IncomingStakingTransactionData::RetireStake {
                        retire_stake: balance,
                        proof: SignatureProof::default(),
                    },
                    validator_key_pair,
                    staker_key_pair,
                }
            }
        }
    }

    /// Creates a new validator with a staker according to the params balance and `validator_state`.
    /// It also inits the staking contract if no staking contract exists, otherwise it modifies the existing one.
    /// Returns the keys for the new validator and staker in the order: `validator_key_pair` (used for all Schnorr keys),
    /// `validator_voting_key_pair` and `staker_key_pair`.
    pub fn create_validator_and_staker(
        &mut self,
        validator_state: ValidatorState,
        staker_inactive: bool,
        staker_retired: bool,
    ) -> (KeyPair, BlsKeyPair, KeyPair) {
        // We create a new account with that balance.
        let validator_key_pair = KeyPair::generate(&mut self.rng);
        let validator_voting_key_pair = BlsKeyPair::generate(&mut self.rng);
        let staker_key_pair = KeyPair::generate(&mut self.rng);

        // Staking account.
        let mut staking_contract = match self
            .accounts
            .get_complete(&Policy::STAKING_CONTRACT_ADDRESS, None)
        {
            Account::Staking(contract) => contract,
            _ => Default::default(),
        };

        // Get the deposit value.
        let deposit = Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT);

        let mut raw_txn = self.accounts.env.write_transaction();
        let mut txn: WriteTransactionProxy = (&mut raw_txn).into();
        let data_store = self.accounts.data_store(&Policy::STAKING_CONTRACT_ADDRESS);
        let mut data_store_write = data_store.write(&mut txn);
        let mut store = StakingContractStoreWrite::new(&mut data_store_write);

        staking_contract
            .create_validator(
                &mut store,
                &Address::from(&validator_key_pair),
                validator_key_pair.public,
                validator_voting_key_pair.public_key.compress(),
                Address::from(&validator_key_pair),
                None,
                deposit,
                None,
                None,
                false,
                &mut TransactionLog::empty(),
            )
            .expect("Failed to create validator");

        match validator_state {
            ValidatorState::Inactive => {
                staking_contract
                    .deactivate_validator(
                        &mut store,
                        &Address::from(&validator_key_pair),
                        &Address::from(&validator_key_pair),
                        0,
                        &mut TransactionLog::empty(),
                    )
                    .expect("Failed to deactivate validator");
            }
            ValidatorState::Retired => {
                staking_contract
                    .retire_validator(
                        &mut store,
                        &Address::from(&validator_key_pair),
                        0,
                        &mut TransactionLog::empty(),
                    )
                    .expect("Failed tor retire validator");
            }
            ValidatorState::Active => {} // Nothing to do here
        }
        let mut data_store_write = data_store.write(&mut txn);
        let mut store = StakingContractStoreWrite::new(&mut data_store_write);

        staking_contract
            .create_staker(
                &mut store,
                &Address::from(&staker_key_pair),
                Coin::from_u64_unchecked(Policy::MINIMUM_STAKE * 2),
                Some(Address::from(&validator_key_pair)),
                Coin::ZERO,
                None,
                &mut TransactionLog::empty(),
            )
            .expect("Failed to create staker");

        if staker_inactive | staker_retired {
            staking_contract
                .set_active_stake(
                    &mut store,
                    &Address::from(&staker_key_pair),
                    Coin::ZERO,
                    0,
                    &mut TransactionLog::empty(),
                )
                .expect("Failed to deactivate stake");
        }
        if staker_retired {
            staking_contract
                .retire_stake(
                    &mut store,
                    &Address::from(&staker_key_pair),
                    Coin::from_u64_unchecked(Policy::MINIMUM_STAKE * 2),
                    Policy::block_after_reporting_window(Policy::election_block_after(0)),
                    &mut TransactionLog::empty(),
                )
                .expect("Failed to deactivate stake");
        }

        self.accounts
            .tree
            .put(
                &mut txn,
                &KeyNibbles::from(&Policy::STAKING_CONTRACT_ADDRESS),
                Account::Staking(staking_contract),
            )
            .expect("Failed to store staking contract");
        self.accounts
            .tree
            .update_root(&mut txn)
            .expect("Tree must be complete");
        raw_txn.commit();

        (
            validator_key_pair,
            validator_voting_key_pair,
            staker_key_pair,
        )
    }
}
