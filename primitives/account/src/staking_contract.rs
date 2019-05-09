use std::cmp::Ordering;
use std::collections::btree_map::BTreeMap;
use std::collections::btree_set::BTreeSet;
use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::Mutex;

use beserial::{Deserialize, ReadBytesExt, Serialize, SerializingError, WriteBytesExt};
use bls::bls12_381::PublicKey as BlsPublicKey;
use keys::Address;
use primitives::coin::Coin;
use transaction::{SignatureProof, Transaction};
use transaction::account::staking_contract::StakingTransactionData;

use crate::{Account, AccountError, AccountType};
use crate::AccountTransactionInteraction;

#[derive(Clone, Debug)]
pub struct ActiveStake {
    staker_address: Address,
    balance: Coin,
    validator_key: BlsPublicKey, // TODO Share validator keys eventually and if required
    reward_address: Option<Address>,
}

impl PartialEq for ActiveStake {
    fn eq(&self, other: &ActiveStake) -> bool {
        self.balance == other.balance
            && self.staker_address == other.staker_address
    }
}

impl Eq for ActiveStake {}

impl PartialOrd for ActiveStake {
    fn partial_cmp(&self, other: &ActiveStake) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ActiveStake {
    fn cmp(&self, other: &Self) -> Ordering {
        self.balance.cmp(&other.balance)
            .then_with(|| self.staker_address.cmp(&other.staker_address))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct InactiveStake {
    balance: Coin,
    retire_time: u32,
}


#[derive(Serialize, Deserialize)]
struct StakeReceipt {
    validator_key: BlsPublicKey,
    reward_address: Option<Address>,
}

#[derive(Serialize, Deserialize)]
struct RetireReceipt {
    validator_key: BlsPublicKey,
    reward_address: Option<Address>,
}

#[derive(Serialize, Deserialize)]
struct UnstakeReceipt {
    retire_time: u32,
}

/*
 Here's an explanation of how the different transactions work.
 1. Stake:
    - Transaction from staking address to contract
    - Transfers value into a new or existing entry in the potential_validators list
    - Normal transaction, signed by staking/sender address
 2. Retire:
    - Transaction from staking contract to itself
    - Removes a balance (the transaction value) from the active stake of a staker
      (may remove staker from active stake list entirely)
    - Puts balance into a new entry into the inactive_stake,
      setting the retire_time for this balance
    - Signed by staking/sender address
 3. Unstake:
    - Transaction from the contract to an external address
    - If condition of block_height ≥ next_macro_block_after(retire_time) + UNSTAKE_DELAY is met,
      transfers value from inactive_validators entry/entries
    - Signed by staking/sender address

  Reverting transactions:
  Since transactions need to be revertable, the with_{incoming,outgoing}_transaction functions
  may also return binary data (Vec<u8>) containing additional information to that transaction.
  Internally, this data can be serialized/deserialized.

  Objects:
  ActiveStake: Active stake is characterized by the tuple
    (staker_address, balance, validator_key, optional reward_address).
  InactiveStake: The information for a single retire transaction, represented by the tuple
    (balance, retire_time).
    If a staker retires multiple times, we need to keep track of the retire_time for every
    retire transaction.

  Internal lookups required:
  - Stake requires a way to get from a staker address to an ActiveStake object
  - Retire requires a way to get from a staker address to an ActiveStake object
    and from a staker address to the list of InactiveStake objects.
  - Unstake requires a way to get from a staker address to the list of InactiveStake objects.
  - Retrieving the list of active stakes that are actually considered for the selection
    requires a list of ActiveStake objects ordered by its balance.
 */
#[derive(Clone, Debug)]
pub struct StakingContract {
    pub balance: Coin,
    pub active_stake_sorted: BTreeSet<Arc<ActiveStake>>, // A list might be sufficient.
    pub active_stake_by_address: HashMap<Address, Arc<ActiveStake>>,
    pub inactive_stake_by_address: HashMap<Address, Vec<InactiveStake>>,
}

impl StakingContract {
    /// Adds funds to stake of `address` and adds `validator_key` if not yet present.
    fn stake(&mut self, staker_address: &Address, value: Coin, validator_key: BlsPublicKey, reward_address: Option<Address>) -> Result<Option<StakeReceipt>, AccountError> {
        self.balance = Account::balance_add(self.balance, value)?;

        if let Some(active_stake) = self.active_stake_by_address.remove(staker_address) {
            let new_active_stake = Arc::new(ActiveStake {
                staker_address: active_stake.staker_address.clone(),
                balance: Account::balance_add(active_stake.balance, value)?,
                validator_key,
                reward_address
            });

            self.active_stake_sorted.remove(&active_stake);
            self.active_stake_sorted.insert(Arc::clone(&new_active_stake));
            self.active_stake_by_address.insert(staker_address.clone(), new_active_stake);

            Ok(Some(StakeReceipt {
                validator_key: active_stake.validator_key.clone(),
                reward_address: active_stake.reward_address.clone(),
            }))
        } else {
            let stake = Arc::new(ActiveStake {
                staker_address: staker_address.clone(),
                balance: value,
                validator_key,
                reward_address,
            });
            self.active_stake_sorted.insert(Arc::clone(&stake));
            self.active_stake_by_address.insert(staker_address.clone(), stake);

            Ok(None)
        }
    }

    /// Reverts a stake transaction.
    fn revert_stake(&mut self, staker_address: &Address, value: Coin, receipt: Option<StakeReceipt>) -> Result<(), AccountError> {
        self.balance = Account::balance_sub(self.balance, value)?;

        let active_stake = self.active_stake_by_address.get(&staker_address);
        if active_stake.is_none() {
            return Err(AccountError::InvalidForRecipient);
        }

        let active_stake = active_stake.unwrap();
        match active_stake.balance.cmp(&value) {
            Ordering::Greater => {
                if receipt.is_none() {
                    return Err(AccountError::InvalidReceipt);
                }

                let receipt = receipt.unwrap();
                let new_active_stake = Arc::new(ActiveStake {
                    staker_address: active_stake.staker_address.clone(),
                    balance: Account::balance_sub(active_stake.balance, value)?,
                    validator_key: receipt.validator_key,
                    reward_address: receipt.reward_address,
                });

                self.active_stake_sorted.remove(active_stake);
                self.active_stake_sorted.insert(Arc::clone(&new_active_stake));
                self.active_stake_by_address.insert(staker_address.clone(), new_active_stake);
                Ok(())
            },
            Ordering::Equal => {
                if receipt.is_some() {
                    return Err(AccountError::InvalidReceipt);
                }

                self.active_stake_sorted.remove(active_stake);
                self.active_stake_by_address.remove(staker_address);
                Ok(())
            },
            Ordering::Less => {
                Err(AccountError::InvalidForRecipient)
            }
        }
    }

    /// Removes stake from the active stake list.
    fn retire(&mut self, staker_address: &Address, value: Coin) -> Result<Option<RetireReceipt>, AccountError> {
        unimplemented!()
    }

    /// Reverts a retire transaction.
    fn revert_retire(&mut self, staker_address: &Address, value: Coin, receipt: Option<RetireReceipt>) -> Result<(), AccountError> {
        unimplemented!()
    }

    /// Removes stake from the inactive stake list.
    fn unstake(&mut self, staker_address: &Address, value: Coin) -> Result<Option<UnstakeReceipt>, AccountError> {
        unimplemented!()
    }

    /// Reverts a unstake transaction.
    fn revert_unstake(&mut self, staker_address: &Address, value: Coin, receipt: Option<UnstakeReceipt>) -> Result<(), AccountError> {
        unimplemented!()
    }

    /// Retrieves the size-bounded list of potential validators.
    /// The algorithm works as follows in psuedo-code:
    // XXX librustdoc test fails if this pseudo-code is in a doc comment.
    // list = sorted list of potential validators (per address) by balances ascending
    // potential_validators = empty list
    // min_stake = 0
    // current_stake = 0
    // loop {
    //     next_validator = list.pop() // Takes highest balance.
    //     if next_validator.balance < min_stake {
    //         break
    //     }
    //
    //     current_stake += next_validator.balance
    //     potential_validators.push(next_validator)
    //     min_stake = current_stake / MAX_POTENTIAL_VALIDATORS
    // }
    pub fn potential_validators(&self) -> Vec<(BlsPublicKey, Coin, Address, Option<Address>)> {
        unimplemented!()
    }
}

impl AccountTransactionInteraction for StakingContract {
    fn new_contract(_: AccountType, _: Coin, _: &Transaction, _: u32) -> Result<Self, AccountError> {
        Err(AccountError::InvalidForRecipient)
    }

    fn create(_: Coin, _: &Transaction, _: u32) -> Result<Self, AccountError> {
        Err(AccountError::InvalidForRecipient)
    }

    fn check_incoming_transaction(&self, transaction: &Transaction, block_height: u32) -> Result<(), AccountError> {
        unimplemented!()
    }

    fn commit_incoming_transaction(&mut self, transaction: &Transaction, block_height: u32) -> Result<Option<Vec<u8>>, AccountError> {
        let data = StakingTransactionData::parse(transaction)?;

        match data {
            StakingTransactionData::Stake { validator_key, reward_address, .. } => {
                Ok(self.stake(&transaction.sender, transaction.value, validator_key, reward_address)?
                    .map(|receipt| receipt.serialize_to_vec()))
            },
            StakingTransactionData::Retire => {
                Ok(self.retire(&transaction.sender, transaction.value)?
                    .map(|receipt| receipt.serialize_to_vec()))
            },
        }
    }

    fn revert_incoming_transaction(&mut self, transaction: &Transaction, _block_height: u32, receipt: Option<&Vec<u8>>) -> Result<(), AccountError> {
        let data = StakingTransactionData::parse(transaction)?;

        match data {
            StakingTransactionData::Stake { .. } => {
                if let Some(receipt) = receipt {
                    let receipt = Deserialize::deserialize_from_vec(&receipt)?;
                    self.revert_stake(&transaction.sender, transaction.value, receipt)
                } else {
                    Err(AccountError::InvalidReceipt)
                }
            },
            StakingTransactionData::Retire => {
                if let Some(receipt) = receipt {
                    let receipt = Deserialize::deserialize_from_vec(&receipt)?;
                    self.revert_retire(&transaction.sender, transaction.value, receipt)
                } else {
                    Err(AccountError::InvalidReceipt)
                }
            },
        }
    }

    fn check_outgoing_transaction(&self, transaction: &Transaction, block_height: u32) -> Result<(), AccountError> {
        unimplemented!()
    }

    fn commit_outgoing_transaction(&mut self, transaction: &Transaction, block_height: u32) -> Result<Option<Vec<u8>>, AccountError> {
        // Remove balance from the inactive list.
        // Returns a receipt containing the pause_times and balances.
        unimplemented!()
    }

    fn revert_outgoing_transaction(&mut self, transaction: &Transaction, _block_height: u32, receipt: Option<&Vec<u8>>) -> Result<(), AccountError> {
        // Needs to reconstruct the previous state by transferring the transaction value back to
        // inactive validator entries using the receipt.
        unimplemented!()
    }
}

impl Serialize for StakingContract {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        unimplemented!()
    }

    fn serialized_size(&self) -> usize {
        unimplemented!()
    }
}

impl Deserialize for StakingContract {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
        unimplemented!()
    }
}

// Not really useful traits for StakingContracts.
impl PartialEq for StakingContract {
    fn eq(&self, other: &StakingContract) -> bool {
        unimplemented!()
    }
}

impl Eq for StakingContract {}

impl PartialOrd for StakingContract {
    fn partial_cmp(&self, other: &StakingContract) -> Option<Ordering> {
        unimplemented!()
    }
}

impl Ord for StakingContract {
    fn cmp(&self, other: &Self) -> Ordering {
        unimplemented!()
    }
}
