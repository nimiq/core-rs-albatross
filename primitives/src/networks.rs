use beserial::{Deserialize, Serialize};
use std::fmt::Display;

#[derive(Serialize, Deserialize, Copy, Clone, Ord, PartialOrd, Eq, PartialEq, Debug, Hash, Display)]
#[repr(u8)]
pub enum NetworkId {
    Test = 1,
    Dev = 2,
    Bounty = 3,
    Dummy = 4,
    Main = 42,

    TestAlbatross = 5,
    DevAlbatross = 6,
}


impl NetworkId {
    pub fn is_albatross(&self) -> bool {
        match self {
            NetworkId::TestAlbatross | NetworkId::DevAlbatross => true,
            _ => false,
        }
    }
}
