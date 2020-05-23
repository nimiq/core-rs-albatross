use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::mem;
use std::ops::{Deref, DerefMut};
use std::sync::Arc;

use parking_lot::RwLock;

use beserial::{
    Deserialize, DeserializeWithLength, ReadBytesExt, Serialize, SerializeWithLength,
    SerializingError, WriteBytesExt,
};
use bls::CompressedPublicKey as BlsPublicKey;
use keys::Address;
use primitives::coin::Coin;

use crate::{Account, AccountError};

#[derive(Debug)]
pub struct Validator {
    pub balance: Coin,
    pub reward_address: Address,
    pub validator_key: BlsPublicKey,
    pub active_stake_by_address: RwLock<BTreeMap<Address, Coin>>,
}

impl Validator {
    pub fn new(
        initial_balance: Coin,
        reward_address: Address,
        validator_key: BlsPublicKey,
    ) -> Self {
        Validator {
            balance: initial_balance,
            reward_address,
            validator_key,
            active_stake_by_address: Default::default(),
        }
    }

    pub fn update_validator(
        &self,
        new_reward_address: Option<Address>,
        new_validator_key: Option<BlsPublicKey>,
    ) -> Self {
        let active_stake_by_address = mem::take(self.active_stake_by_address.write().deref_mut());
        Validator {
            balance: self.balance,
            reward_address: new_reward_address.unwrap_or_else(|| self.reward_address.clone()),
            validator_key: new_validator_key.unwrap_or_else(|| self.validator_key.clone()),
            active_stake_by_address: RwLock::new(active_stake_by_address),
        }
    }

    fn with_balance(&self, balance: Coin) -> Self {
        let active_stake_by_address = mem::take(self.active_stake_by_address.write().deref_mut());
        Validator {
            balance,
            reward_address: self.reward_address.clone(),
            validator_key: self.validator_key.clone(),
            active_stake_by_address: RwLock::new(active_stake_by_address),
        }
    }

    pub fn add_stake(&self, staker_address: Address, stake: Coin) -> Result<Self, AccountError> {
        let new_balance = Account::balance_add(self.balance, stake)?;
        let validator = self.with_balance(new_balance);

        // We do not need to check for overflows here, because self.balance is always larger.
        *validator
            .active_stake_by_address
            .write()
            .entry(staker_address)
            .or_insert(Coin::ZERO) += stake;
        Ok(validator)
    }

    pub fn sub_stake(
        &self,
        staker_address: &Address,
        value: Coin,
        not_present_error: AccountError,
    ) -> Result<Self, AccountError> {
        // First update stake entry.
        let mut active_stake_by_address = self.active_stake_by_address.write();

        if let Some(stake) = active_stake_by_address.get_mut(staker_address) {
            *stake = Account::balance_sub(*stake, value)?;

            if stake.is_zero() {
                active_stake_by_address.remove(staker_address);
            }
        } else {
            return Err(not_present_error);
        }
        drop(active_stake_by_address);

        let new_balance = Account::balance_sub(self.balance, value)?;
        let validator = self.with_balance(new_balance);
        Ok(validator)
    }
}

impl Serialize for Validator {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        let mut size = 0;
        size += Serialize::serialize(&self.balance, writer)?;
        size += Serialize::serialize(&self.reward_address, writer)?;
        size += Serialize::serialize(&self.validator_key, writer)?;
        size += SerializeWithLength::serialize::<u32, _>(
            self.active_stake_by_address.read().deref(),
            writer,
        )?;
        Ok(size)
    }

    fn serialized_size(&self) -> usize {
        let mut size = 0;
        size += Serialize::serialized_size(&self.balance);
        size += Serialize::serialized_size(&self.reward_address);
        size += Serialize::serialized_size(&self.validator_key);
        size += SerializeWithLength::serialized_size::<u32>(
            self.active_stake_by_address.read().deref(),
        );
        size
    }
}

impl Deserialize for Validator {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
        let balance = Deserialize::deserialize(reader)?;
        let reward_address = Deserialize::deserialize(reader)?;
        let validator_key = Deserialize::deserialize(reader)?;
        let active_stake_by_address: BTreeMap<Address, Coin> =
            DeserializeWithLength::deserialize::<u32, _>(reader)?;
        Ok(Validator {
            balance,
            reward_address,
            validator_key,
            active_stake_by_address: RwLock::new(active_stake_by_address),
        })
    }
}

impl PartialEq for Validator {
    fn eq(&self, other: &Validator) -> bool {
        self.validator_key == other.validator_key && self.balance == other.balance
    }
}

impl Eq for Validator {}

impl PartialOrd for Validator {
    fn partial_cmp(&self, other: &Validator) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Validator {
    // Highest to low balances
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .balance
            .cmp(&self.balance)
            .then_with(|| self.validator_key.cmp(&other.validator_key))
    }
}

impl Clone for Validator {
    fn clone(&self) -> Self {
        Validator {
            balance: self.balance,
            reward_address: self.reward_address.clone(),
            validator_key: self.validator_key.clone(),
            active_stake_by_address: RwLock::new(self.active_stake_by_address.read().clone()),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct InactiveValidator {
    pub validator: Arc<Validator>,
    pub retire_time: u32,
}

impl Clone for InactiveValidator {
    fn clone(&self) -> Self {
        InactiveValidator {
            // Do not just copy Arc!
            validator: Arc::new(self.validator.as_ref().clone()),
            retire_time: self.retire_time,
        }
    }
}

/// A validator entry combines active and inactive validators.
/// It also allows trying to modify an entry with deferred error handling.
pub enum ValidatorEntry {
    Active(Arc<Validator>, Option<AccountError>),
    Inactive(InactiveValidator, Option<AccountError>),
}

impl ValidatorEntry {
    pub fn new_active_validator(validator: Arc<Validator>) -> Self {
        ValidatorEntry::Active(validator, None)
    }

    pub fn new_inactive_validator(validator: InactiveValidator) -> Self {
        ValidatorEntry::Inactive(validator, None)
    }

    pub fn as_validator(&self) -> &Arc<Validator> {
        match self {
            ValidatorEntry::Active(validator, _) => validator,
            ValidatorEntry::Inactive(validator, _) => &validator.validator,
        }
    }

    fn replace(&mut self, potential_validator: Result<Arc<Validator>, AccountError>) {
        match potential_validator {
            Ok(new_validator) => match self {
                ValidatorEntry::Active(ref mut validator, _) => {
                    *validator = new_validator;
                }
                ValidatorEntry::Inactive(ref mut validator, _) => {
                    validator.validator = new_validator;
                }
            },
            Err(err) => match self {
                ValidatorEntry::Active(_, ref mut error) => {
                    error.replace(err);
                }
                ValidatorEntry::Inactive(_, ref mut error) => {
                    error.replace(err);
                }
            },
        }
    }

    pub fn update_validator(
        &mut self,
        new_reward_address: Option<Address>,
        new_validator_key: Option<BlsPublicKey>,
    ) {
        self.replace(Ok(Arc::new(
            self.as_validator()
                .update_validator(new_reward_address, new_validator_key),
        )))
    }

    /// This function will only change the validator entry if add_stake is successful.
    pub fn try_add_stake(&mut self, staker_address: Address, stake: Coin) {
        let new_validator = self.as_validator().add_stake(staker_address, stake);
        self.replace(new_validator.map(Arc::new));
    }

    /// This function will only change the validator entry if sub_stake is successful.
    pub fn try_sub_stake(
        &mut self,
        staker_address: &Address,
        value: Coin,
        not_present_error: AccountError,
    ) {
        let new_validator = self
            .as_validator()
            .sub_stake(staker_address, value, not_present_error);
        self.replace(new_validator.map(Arc::new));
    }
}
