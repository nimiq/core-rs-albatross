use std::fmt::Display;

use enum_display_derive::Display;

use beserial::{Deserialize, Serialize};

#[derive(Clone, Copy, PartialEq, PartialOrd, Eq, Ord, Debug, Serialize, Deserialize, Display)]
#[repr(u8)]
pub enum AccountType {
    Basic = 0,
    Vesting = 1,
    HTLC = 2,
    Staking = 3,
}

impl AccountType {
    pub fn from_int(x: u8) -> Option<AccountType> {
        match x {
            0 => Some(AccountType::Basic),
            1 => Some(AccountType::Vesting),
            2 => Some(AccountType::HTLC),
            3 => Some(AccountType::Staking),
            _ => None,
        }
    }
}
