use nimiq_keys::Address;
use nimiq_primitives::account::AccountError;
use nimiq_primitives::coin::Coin;
use std::collections::BTreeMap;

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
        self.reserve_for(&self.address, available, amount)
    }

    pub fn reserve_unchecked(&mut self, amount: Coin) -> Result<(), AccountError> {
        self.reserve_for(&self.address, Coin::MAX, amount)
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

    pub fn balance(&self) -> Coin {
        self.balance_for(&self.address)
    }

    pub fn balance_for(&self, address: &Address) -> Coin {
        self.balances.get(address).cloned().unwrap_or(Coin::ZERO)
    }
}
