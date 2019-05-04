use std::cmp::Ordering;
use std::collections::btree_map::BTreeMap;
use std::collections::btree_set::BTreeSet;
use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::Mutex;

use beserial::{Deserialize, ReadBytesExt, Serialize, SerializingError, WriteBytesExt};
use keys::Address;
use nimiq_bls::bls12_381::{PublicKey as BlsPublicKey, PublicKeyAffine as BlsPublicKeyAffine};
use nimiq_collections::SparseVec;
use primitives::coin::Coin;
use primitives::policy;
use transaction::{SignatureProof, Transaction};
use transaction::account::{parse_and_verify_staking_transaction, StakingTransactionData, StakingTransactionType};

use crate::AccountTransactionInteraction;

use super::{Account, AccountError, AccountType};

#[derive(Clone, Debug)]
pub struct PotentialValidator {
    staking_address: Address,
    payout_address: Option<Address>,
    balance: Coin,
    key_info: Arc<Mutex<KeyInfo>>,
}

impl PotentialValidator {
    fn with_balance(&self, balance: Coin) -> Self {
        PotentialValidator {
            staking_address: self.staking_address.clone(),
            payout_address: self.payout_address.clone(),
            balance,
            key_info: self.key_info.clone(),
        }
    }
}

impl PartialEq for PotentialValidator {
    fn eq(&self, other: &PotentialValidator) -> bool {
        self.balance == other.balance
            && self.staking_address == other.staking_address
    }
}

impl Eq for PotentialValidator {}

impl PartialOrd for PotentialValidator {
    fn partial_cmp(&self, other: &PotentialValidator) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for PotentialValidator {
    fn cmp(&self, other: &Self) -> Ordering {
        self.balance.cmp(&other.balance)
            .then_with(|| self.staking_address.cmp(&other.staking_address))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct KeyInfo {
    last_restaking: u32,
    validator_key: BlsPublicKey,
}

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct PausingTransaction {
    balance: Coin,
    pause_time: u32,
}

#[derive(Serialize, Deserialize)]
struct RestakingReceipt {
    prev_last_restaking: u32,
}

#[derive(Serialize, Deserialize)]
struct PausingReceipt {
}

/*
 Here's an explanation of how the different transactions work.
 1. Staking:
    - Transaction from staking address to contract
    - Transfers value into a new or existing entry in the potential_validators list
    - Normal transaction, signed by staking/sender address
 2. Restaking:
    - Transaction with 0 value from any address to contract (address only provides fees)
    - Replaces a BlsPublicKey and sets the last_restaking time
    - Transaction is signed by sender address that provides the fee
    - Data is additionally signed by the old BLS key
 3. Pausing:
    - Transaction with 0 value from original staking address to contract
    - Removes a balance (set in transaction data) from a staker
      (may remove staker from potential_validators list entirely)
    - Puts balance into a new entry into the inactive_validators,
      setting the pause_time for this balance
    - Signed by staking/sender address
 4. Unstaking:
    - Transaction from the contract to an original staking address
    - If condition of block_height ≥ next_macro_block_after(pause_time) + UNSTAKING_DELAY is met,
      transfers value from inactive_validators entry/entries
    - Signed by staking/sender address

  Reverting transactions:
  Since transactions need to be revertable, the with_{incoming,outgoing}_transaction functions
  may also return binary data (Vec<u8>) containing additional information to that transaction.
  Internally, this data can be serialized/deserialized.

  Objects:
  PotentialValidator: A potential validator is characterized by the tuple
    (staking_address, balance, optional payout_address, validator_key_id).
    It makes sense to store validator keys separately, to allow easy restaking transactions without
    scanning all uses of the same validator key.
  KeyInfo: The separate information on the validator key, represented by the tuple:
    (validator_key, last_restaking).
  PausingTransaction: The information for a single pause transaction, represented by the tuple
    (balance, pause_time).
    If a validator pauses multiple times, we need to keep track of the pause_time for every
    pausing transaction.

  Internal lookups required:
  - Staking requires a way to get from a staking address to a PotentialValidator object.
  - Restaking requires a way to get from an old BlsPublicKey to a KeyInfo object.
  - Pausing requires a way to get from a staking address to a PotentialValidator object
    and from a staking address to the list of PausingTransaction objects.
  - Unstaking requires a way to get from a staking address to the list of PausintTransaction objects.
  - Retrieving the list of potential validators that are actually considered for the selection
    requires a list of PotentialValidator objects ordered by its balance.
 */
#[derive(Clone, Debug)]
pub struct StakingContract {
    pub balance: Coin,
    pub potential_validators: BTreeSet<Arc<PotentialValidator>>, // A list might be sufficient.
    pub validators_by_address: HashMap<Address, Arc<PotentialValidator>>,
    pub keys: BTreeMap<BlsPublicKeyAffine, Arc<Mutex<KeyInfo>>>, // TODO: HashMap is not yet possible, since Hash trait is missing. Implement Hash for PublicKeyAffine.
    pub inactive_validators: HashMap<Address, Vec<PausingTransaction>>,
}

impl StakingContract {
    /// Adds funds to stake of `address` and adds `validator_key` if not yet present.
    fn stake(&mut self, address: Address, validator_key: BlsPublicKey, payout_address: Option<Address>,
             balance: Coin, block_height: u32) -> Result<(), AccountError> {
        let validator_key_affine = BlsPublicKeyAffine::from(validator_key.clone());

        // Add or get key info.
        let key_info = self.keys.entry(validator_key_affine).or_insert_with(|| Arc::new(Mutex::new(KeyInfo {
            last_restaking: block_height,
            validator_key,
        }))).clone();

        // Add funds to stake.
        if let Some(validator_data) = self.validators_by_address.remove(&address) {
            self.potential_validators.remove(&validator_data);

            // Only accept with same validator key.
            if validator_data.key_info.lock().ne(&key_info.lock()) {
                return Err(AccountError::InvalidForRecipient);
            }

            // Never allow overwriting the payout address without unstaking.
            if payout_address.is_some() && validator_data.payout_address != payout_address {
                return Err(AccountError::InvalidForRecipient);
            }

            let new_validator_data = Arc::new(validator_data
                .with_balance(validator_data.balance.checked_add(balance)
                    .ok_or(AccountError::InvalidCoinValue)?));

            self.potential_validators.insert(new_validator_data.clone());
            self.validators_by_address.insert(address, new_validator_data);
        } else {
            let validator = Arc::new(PotentialValidator {
                balance,
                payout_address,
                key_info,
                staking_address: address.clone(),
            });

            self.potential_validators.insert(validator.clone());
            self.validators_by_address.insert(address, validator);
        }

        self.balance.checked_add(balance)
            .ok_or(AccountError::InvalidCoinValue)?;

        Ok(())
    }

    /// Reverts a staking transaction.
    fn revert_stake(&mut self, address: Address, validator_key: BlsPublicKey,
             balance: Coin) -> Result<(), AccountError> {
        unimplemented!()
    }

    /// Replace validator key and update `last_restaking`. Also produces receipt.
    fn restake(&mut self, old_validator_key: BlsPublicKey, new_validator_key: BlsPublicKey,
               block_height: u32) -> Result<RestakingReceipt, AccountError> {
        let old_validator_key_affine = BlsPublicKeyAffine::from(old_validator_key);
        let new_validator_key_affine = BlsPublicKeyAffine::from(new_validator_key.clone());

        // Remove old index.
        if let Some(key_info) = self.keys.remove(&old_validator_key_affine) {
            let receipt;
            {
                let mut key_info_guard = key_info.lock();
                // Produce receipt.
                receipt = RestakingReceipt {
                    prev_last_restaking: key_info_guard.last_restaking,
                };

                // Update values.
                key_info_guard.last_restaking = block_height;
                key_info_guard.validator_key = new_validator_key;
            }

            // Add new index.
            self.keys.insert(new_validator_key_affine, key_info);

            Ok(receipt)
        } else {
            Err(AccountError::InvalidForRecipient)
        }
    }

    /// Reverts a restaking.
    fn revert_restake(&mut self, old_validator_key: BlsPublicKey, new_validator_key: BlsPublicKey,
               receipt: RestakingReceipt) -> Result<(), AccountError> {
        let old_validator_key_affine = BlsPublicKeyAffine::from(old_validator_key.clone());
        let new_validator_key_affine = BlsPublicKeyAffine::from(new_validator_key);

        // Remove new index.
        if let Some(key_info) = self.keys.remove(&new_validator_key_affine) {
            {
                let mut key_info_guard = key_info.lock();

                // Update values.
                key_info_guard.last_restaking = receipt.prev_last_restaking;
                key_info_guard.validator_key = old_validator_key;
            }

            // Add new index.
            self.keys.insert(old_validator_key_affine, key_info);

            Ok(())
        } else {
            Err(AccountError::InvalidForRecipient)
        }
    }

    /// Removes stake from the potential validator set.
    fn pause(&mut self, address: Address, balance: Coin) -> Result<PausingReceipt, AccountError> {
        unimplemented!()
    }

    /// Removes stake from the potential validator set.
    fn revert_pause(&mut self, address: Address, balance: Coin, receipt: PausingReceipt) -> Result<(), AccountError> {
        unimplemented!()
    }

    /// Retrieves the size-bounded list of potential validators.
    /// The algorithm works as follows in psuedo-code:
    /// list = sorted list of potential validators (per address) by balances ascending
    /// potential_validators = empty list
    /// min_stake = 0
    /// current_stake = 0
    /// loop {
    ///     next_validator = list.pop() // Takes highest balance.
    ///     if next_validator.balance < min_stake {
    ///         break;
    ///     }
    ///
    ///     current_stake += next_validator.balance
    ///     potential_validators.push(next_validator)
    ///     min_stake = current_stake / MAX_POTENTIAL_VALIDATORS
    /// }
    pub fn potential_validators(&self) -> Vec<(BlsPublicKey, Address)> {
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

    fn with_incoming_transaction(&self, transaction: &Transaction, block_height: u32) -> Result<(Self, Option<Vec<u8>>), AccountError> {
        let data = parse_and_verify_staking_transaction(transaction)?;
        let address = transaction.sender.clone();

        let mut contract = self.clone();
        let mut receipt;

        match data {
            StakingTransactionData::Staking { validator_key, payout_address, .. } => {
                contract.stake(address, validator_key,
                               payout_address, transaction.value, block_height)?;
                receipt = None;
            },
            StakingTransactionData::Restaking { old_validator_key, new_validator_key, .. } => {
                receipt = Some(contract.restake(old_validator_key, new_validator_key, block_height)?
                    .serialize_to_vec());
            },
            StakingTransactionData::Pausing { balance } => {
                receipt = Some(contract.pause(address, balance)?
                    .serialize_to_vec());
            },
        }

        Ok((contract, receipt))
    }

    fn without_incoming_transaction(&self, transaction: &Transaction, _block_height: u32, receipt: Option<Vec<u8>>) -> Result<Self, AccountError> {
        let data = parse_and_verify_staking_transaction(transaction)?;
        let address = transaction.sender.clone();

        let mut contract = self.clone();

        match data {
            StakingTransactionData::Staking { validator_key, .. } => {
                contract.revert_stake(address, validator_key,transaction.value)?;
            },
            StakingTransactionData::Restaking { old_validator_key, new_validator_key, .. } => {
                if let Some(mut receipt) = receipt {
                    let receipt = Deserialize::deserialize_from_vec(&receipt)?;
                    contract.revert_restake(old_validator_key, new_validator_key, receipt)?;
                } else {
                    return Err(AccountError::InvalidPruning);
                }
            },
            StakingTransactionData::Pausing { balance } => {
                if let Some(mut receipt) = receipt {
                    let receipt = Deserialize::deserialize_from_vec(&receipt)?;
                    contract.revert_pause(address, balance, receipt)?;
                } else {
                    return Err(AccountError::InvalidPruning);
                }
            },
        }

        Ok(contract)
    }

    fn with_outgoing_transaction(&self, transaction: &Transaction, block_height: u32) -> Result<(Self, Option<Vec<u8>>), AccountError> {
        // Remove balance from the inactive list.
        // Returns a receipt containing the pause_times and balances.
        unimplemented!()
    }

    fn without_outgoing_transaction(&self, transaction: &Transaction, _block_height: u32, receipt: Option<Vec<u8>>) -> Result<Self, AccountError> {
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