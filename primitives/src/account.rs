use hex::FromHex;

use beserial::{Deserialize, Serialize};

#[derive(Clone, Copy, PartialEq, PartialOrd, Eq, Ord, Debug, Serialize, Deserialize)]
#[repr(u8)]
pub enum AccountType {
    Basic = 0,
    Vesting = 1,
    HTLC = 2,
}

impl AccountType {
    pub fn from_int(x: u8) -> Option<AccountType> {
        match x {
            0 => Some(AccountType::Basic),
            1 => Some(AccountType::Vesting),
            2 => Some(AccountType::HTLC),
            _ => None
        }
    }
}

#[derive(Clone, Copy, PartialEq, PartialOrd, Eq, Ord, Debug, Serialize, Deserialize)]
#[repr(u8)]
pub enum HashAlgorithm {
    Blake2b = 1,
    Sha256 = 3
}

#[derive(Clone, Copy, PartialEq, PartialOrd, Eq, Ord, Debug, Serialize, Deserialize)]
#[repr(u8)]
pub enum ProofType {
    RegularTransfer = 1,
    EarlyResolve = 2,
    TimeoutResolve = 3
}

create_typed_array!(AnyHash, u8, 32);
add_hex_io_fns_typed_arr!(AnyHash, AnyHash::SIZE);
