use std::collections::BTreeMap;

use nimiq_keys::Address;
use nimiq_primitives::{account::AccountError, coin::Coin};

#[derive(Clone)]
pub struct ReservedBalance {
    address: Address,
    balances: BTreeMap<Address, Coin>,
}

impl ReservedBalance {
    pub fn new(address: Address) -> Self {
        Self {
            address,
            balances: BTreeMap::new(),
        }
    }

    pub fn reserve(&mut self, available: Coin, amount: Coin) -> Result<(), AccountError> {
        self.reserve_for(&self.address.clone(), available, amount)
    }

    pub fn reserve_for(
        &mut self,
        address: &Address,
        available: Coin,
        amount: Coin,
    ) -> Result<(), AccountError> {
        let reserved = self.balances.get(address).unwrap_or(&Coin::ZERO);
        let needed = reserved
            .checked_add(amount)
            .ok_or(AccountError::InvalidCoinValue)?;
        if needed > available {
            return Err(AccountError::InsufficientFunds {
                balance: available,
                needed,
            });
        }

        self.balances.insert(address.clone(), needed);
        Ok(())
    }

    pub fn release(&mut self, amount: Coin) {
        self.release_for(&self.address.clone(), amount)
    }

    pub fn release_for(&mut self, address: &Address, amount: Coin) {
        let reserved = self.balances.get(address).unwrap_or(&Coin::ZERO);
        let reserved = reserved.checked_sub(amount).unwrap_or_else(|| {
            panic!("Tried to release more than was reserved ({amount} > {reserved})")
        });
        self.balances.insert(address.clone(), reserved);
    }

    pub fn balance(&self) -> Coin {
        self.balance_for(&self.address)
    }

    pub fn balance_for(&self, address: &Address) -> Coin {
        self.balances.get(address).cloned().unwrap_or(Coin::ZERO)
    }
}
