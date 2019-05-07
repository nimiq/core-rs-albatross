use std::cmp::Ordering;
use std::collections::btree_map::BTreeMap;
use std::collections::btree_set::BTreeSet;
use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::Mutex;

use beserial::{Deserialize, ReadBytesExt, Serialize, SerializingError, WriteBytesExt};
use keys::Address;
use nimiq_bls::bls12_381::{PublicKey as BlsPublicKey, PublicKeyAffine as BlsPublicKeyAffine};
use primitives::coin::Coin;
use transaction::{SignatureProof, Transaction};
use transaction::account::{parse_and_verify_staking_transaction, StakingTransactionData, StakingTransactionType};

use crate::AccountTransactionInteraction;

use super::{Account, AccountError, AccountType};

#[derive(Clone, Debug)]
pub struct ActiveStake {
    balance: Coin,
    staking_address: Address,
    payout_address: Option<Address>,
    validator_info: Arc<Mutex<ValidatorInfo>>,
}

impl ActiveStake {
    fn with_balance(&self, balance: Coin) -> Self {
        ActiveStake {
            balance,
            staking_address: self.staking_address.clone(),
            payout_address: self.payout_address.clone(),
            validator_info: Arc::clone(&self.validator_info),
        }
    }
}

impl PartialEq for ActiveStake {
    fn eq(&self, other: &ActiveStake) -> bool {
        self.balance == other.balance
            && self.staking_address == other.staking_address
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
            .then_with(|| self.staking_address.cmp(&other.staking_address))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct ValidatorInfo {
    public_key: BlsPublicKey,
    last_keepalive: u32,
    #[beserial(skip)]
    num_stakes: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct InactiveStake {
    balance: Coin,
    retire_time: u32,
}

#[derive(Serialize, Deserialize)]
struct KeepaliveReceipt {
    last_keepalive: u32,
}

#[derive(Serialize, Deserialize)]
struct RetireReceipt {
}

/*
 Here's an explanation of how the different transactions work.
 1. Stake:
    - Transaction from staking address to contract
    - Transfers value into a new or existing entry in the potential_validators list
    - Normal transaction, signed by staking/sender address
 2. Keepalive:
    - Transaction with 0 value from any address to contract (address only provides fees)
    - Replaces a BlsPublicKey and sets the last_restaking time
    - Transaction is signed by sender address that provides the fee
    - Data is additionally signed by the old BLS key
 3. Retire:
    - Transaction with 0 value from original staking address to contract
    - Removes a balance (set in transaction data) from a staker
      (may remove staker from active_stake list entirely)
    - Puts balance into a new entry into the inactive_stake,
      setting the retire_time for this balance
    - Signed by staking/sender address
 4. Unstake:
    - Transaction from the contract to an external address
    - If condition of block_height ≥ next_macro_block_after(retire_time) + UNSTAKE_DELAY is met,
      transfers value from inactive_validators entry/entries
    - Signed by staking/sender address

  Reverting transactions:
  Since transactions need to be revertable, the with_{incoming,outgoing}_transaction functions
  may also return binary data (Vec<u8>) containing additional information to that transaction.
  Internally, this data can be serialized/deserialized.

  Objects:
  ActiveStake: Active stake is characterized by the tuple:
    (balance, staking_address, optional payout_address, validator_key_id).
    It makes sense to store validator keys separately, to allow easy keepalive transactions without
    scanning all uses of the same validator key.
  InactiveStake: The information for a single retire transaction, represented by the tuple
    (balance, retire_time).
    If a staker retires multiple times, we need to keep track of the retire_time for every
    retire transaction.
  ValidatorInfo: The separate information on the validator key, represented by the tuple:
    (validator_key, last_keepalive).

  Internal lookups required:
  - Stake requires a way to get from a staking address to an ActiveStake object.
  - Keepalive requires a way to get from an old BlsPublicKey to a ValidatorInfo object.
  - Retire requires a way to get from a staking address to an ActiveStake object
    and from a staking address to the list of InactiveStake objects.
  - Unstake requires a way to get from a staking address to the list of InactiveStake objects.
  - Retrieving the list of active stakes that are actually considered for the selection
    requires a list of ActiveStake objects ordered by its balance.
 */
#[derive(Clone, Debug)]
pub struct StakingContract {
    pub balance: Coin,
    pub active_stake: BTreeSet<Arc<ActiveStake>>, // A list might be sufficient.
    pub stake_by_address: HashMap<Address, Arc<ActiveStake>>,
    pub inactive_stake: HashMap<Address, Vec<InactiveStake>>,
    pub validator_infos: BTreeMap<BlsPublicKeyAffine, Arc<Mutex<ValidatorInfo>>>, // TODO: HashMap is not yet possible, since Hash trait is missing. Implement Hash for PublicKeyAffine.
}

impl StakingContract {
    /// Adds funds to stake of `address` and adds `validator_key` if not yet present.
    fn stake(&mut self, address: Address, validator_key: BlsPublicKey, payout_address: Option<Address>,
             balance: Coin, block_height: u32) -> Result<(), AccountError> {
        if let Some(validator_data) = self.stake_by_address.get(&address) {
            // Only accept with same validator key.
            if validator_data.validator_info.lock().public_key != validator_key {
                return Err(AccountError::InvalidForRecipient);
            }

            // Never allow overwriting the payout address without unstaking.
            // XXX Why would we this be required?
            if payout_address.is_some() && validator_data.payout_address != payout_address {
                return Err(AccountError::InvalidForRecipient);
            }

            let new_validator_data = Arc::new(validator_data
                .with_balance(validator_data.balance.checked_add(balance)
                    .ok_or(AccountError::InvalidCoinValue)?));

            self.active_stake.remove(validator_data);
            self.active_stake.insert(new_validator_data.clone());
            self.stake_by_address.insert(address, new_validator_data);
        } else {
            let validator_key_affine = BlsPublicKeyAffine::from(validator_key.clone());
            let validator_info = Arc::clone(self.validator_infos
                .entry(validator_key_affine)
                .or_insert_with(|| Arc::new(Mutex::new(ValidatorInfo {
                    public_key: validator_key,
                    last_keepalive: block_height,
                    num_stakes: 0
                }))));
            validator_info.lock().num_stakes += 1;

            let stake = Arc::new(ActiveStake {
                balance,
                payout_address,
                validator_info,
                staking_address: address.clone(),
            });
            self.active_stake.insert(Arc::clone(&stake));
            self.stake_by_address.insert(address, stake);
        }

        self.balance.checked_add(balance)
            .ok_or(AccountError::InvalidCoinValue)?;

        Ok(())
    }

    /// Reverts a staking transaction.
    fn revert_stake(&mut self, address: Address, validator_key: BlsPublicKey,
             balance: Coin) -> Result<(), AccountError> {
        if let Some(validator_data) = self.stake_by_address.get(&address) {
            match validator_data.balance.cmp(&balance) {
                Ordering::Greater => {
                    // TODO what do want to check here?

                    let new_validator_data = Arc::new(validator_data
                        .with_balance(validator_data.balance.checked_sub(balance)
                            .ok_or(AccountError::InvalidCoinValue)?));

                    self.active_stake.remove(validator_data);
                    self.active_stake.insert(new_validator_data.clone());
                    self.stake_by_address.insert(address, new_validator_data);
                    Ok(())
                },
                Ordering::Equal => {
                    unimplemented!()
                },
                Ordering::Less => {
                    Err(AccountError::InvalidForRecipient)
                }
            }
        } else {
            Err(AccountError::InvalidForRecipient)
        }
    }

    /// Replace validator key and update `last_keepalive`. Also produces receipt.
    fn keepalive(&mut self, old_validator_key: BlsPublicKey, new_validator_key: BlsPublicKey,
                 block_height: u32) -> Result<KeepaliveReceipt, AccountError> {
        let old_validator_key_affine = BlsPublicKeyAffine::from(old_validator_key);
        let new_validator_key_affine = BlsPublicKeyAffine::from(new_validator_key.clone());

        // Remove old index.
        if let Some(validator_info) = self.validator_infos.remove(&old_validator_key_affine) {
            let receipt;
            {
                let mut info_locked = validator_info.lock();

                // Produce receipt.
                receipt = KeepaliveReceipt {
                    last_keepalive: info_locked.last_keepalive,
                };

                // Update values.
                info_locked.public_key = new_validator_key;
                info_locked.last_keepalive = block_height;
            }

            // Add new index.
            self.validator_infos.insert(new_validator_key_affine, validator_info);

            Ok(receipt)
        } else {
            Err(AccountError::InvalidForRecipient)
        }
    }

    /// Reverts a keepalive.
    fn revert_keepalive(&mut self, old_validator_key: BlsPublicKey, new_validator_key: BlsPublicKey,
                        receipt: KeepaliveReceipt) -> Result<(), AccountError> {
        let old_validator_key_affine = BlsPublicKeyAffine::from(old_validator_key.clone());
        let new_validator_key_affine = BlsPublicKeyAffine::from(new_validator_key);

        // Remove new index.
        if let Some(validator_info) = self.validator_infos.remove(&new_validator_key_affine) {
            {
                let mut info_locked = validator_info.lock();

                // Update values.
                info_locked.public_key = old_validator_key;
                info_locked.last_keepalive = receipt.last_keepalive;
            }

            // Add new index.
            self.validator_infos.insert(old_validator_key_affine, validator_info);

            Ok(())
        } else {
            Err(AccountError::InvalidForRecipient)
        }
    }

    /// Removes stake from the active stake list.
    fn retire(&mut self, address: Address, balance: Coin) -> Result<RetireReceipt, AccountError> {
        unimplemented!()
    }

    /// Removes stake from the potential validator set.
    fn revert_retire(&mut self, address: Address, balance: Coin, receipt: RetireReceipt) -> Result<(), AccountError> {
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
        let data = parse_and_verify_staking_transaction(transaction)?;
        let address = transaction.sender.clone();

        let mut receipt;
        match data {
            StakingTransactionData::Staking { validator_key, payout_address, .. } => {
                self.stake(address, validator_key,
                               payout_address, transaction.value, block_height)?;
                receipt = None;
            },
            StakingTransactionData::Restaking { old_validator_key, new_validator_key, .. } => {
                receipt = Some(self.keepalive(old_validator_key, new_validator_key, block_height)?
                    .serialize_to_vec());
            },
            StakingTransactionData::Pausing { balance } => {
                receipt = Some(self.retire(address, balance)?
                    .serialize_to_vec());
            },
        }

        Ok(receipt)
    }

    fn revert_incoming_transaction(&mut self, transaction: &Transaction, _block_height: u32, receipt: Option<&Vec<u8>>) -> Result<(), AccountError> {
        let data = parse_and_verify_staking_transaction(transaction)?;
        let address = transaction.sender.clone();

        match data {
            StakingTransactionData::Staking { validator_key, .. } => {
                self.revert_stake(address, validator_key,transaction.value)?;
            },
            StakingTransactionData::Restaking { old_validator_key, new_validator_key, .. } => {
                if let Some(mut receipt) = receipt {
                    let receipt = Deserialize::deserialize_from_vec(&receipt)?;
                    self.revert_keepalive(old_validator_key, new_validator_key, receipt)?;
                } else {
                    return Err(AccountError::InvalidReceipt);
                }
            },
            StakingTransactionData::Pausing { balance } => {
                if let Some(mut receipt) = receipt {
                    let receipt = Deserialize::deserialize_from_vec(&receipt)?;
                    self.revert_retire(address, balance, receipt)?;
                } else {
                    return Err(AccountError::InvalidReceipt);
                }
            },
        }

        Ok(())
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
