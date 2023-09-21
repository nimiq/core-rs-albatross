use std::{
    fmt::{Display, Error, Formatter},
    str::FromStr,
};

use thiserror::Error;

#[derive(Copy, Clone, Ord, PartialOrd, Eq, PartialEq, Debug, Hash)]
#[cfg_attr(
    feature = "serde-derive",
    derive(serde_repr::Serialize_repr, serde_repr::Deserialize_repr)
)]
#[repr(u8)]
pub enum NetworkId {
    Test = 1,
    Dev = 2,
    Bounty = 3,
    Dummy = 4,
    Main = 42,

    TestAlbatross = 5,
    DevAlbatross = 6,
    UnitAlbatross = 7,
    MainAlbatross = 24,
}

impl NetworkId {
    pub fn is_albatross(self) -> bool {
        matches!(
            self,
            NetworkId::TestAlbatross
                | NetworkId::DevAlbatross
                | NetworkId::UnitAlbatross
                | NetworkId::MainAlbatross
        )
    }
}

impl Default for NetworkId {
    fn default() -> Self {
        Self::DevAlbatross
    }
}

#[derive(Error, Debug)]
#[error("Input is not a valid network name: {0}")]
pub struct NetworkIdParseError(String);

impl FromStr for NetworkId {
    type Err = NetworkIdParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "test" => Ok(NetworkId::Test),
            "dev" => Ok(NetworkId::Dev),
            "bounty" => Ok(NetworkId::Bounty),
            "dummy" => Ok(NetworkId::Dummy),
            "main" => Ok(NetworkId::Main),
            "unitalbatross" => Ok(NetworkId::UnitAlbatross),
            "testalbatross" => Ok(NetworkId::TestAlbatross),
            "devalbatross" => Ok(NetworkId::DevAlbatross),
            "mainalbatross" => Ok(NetworkId::MainAlbatross),
            _ => Err(NetworkIdParseError(String::from(s))),
        }
    }
}

impl Display for NetworkId {
    fn fmt(&self, f: &mut Formatter) -> Result<(), Error> {
        f.write_str(match *self {
            NetworkId::Test => "Test",
            NetworkId::Dev => "Dev",
            NetworkId::Bounty => "Bounty",
            NetworkId::Dummy => "Dummy",
            NetworkId::Main => "Main",
            NetworkId::TestAlbatross => "TestAlbatross",
            NetworkId::DevAlbatross => "DevAlbatross",
            NetworkId::UnitAlbatross => "UnitAlbatross",
            NetworkId::MainAlbatross => "MainAlbatross",
        })
    }
}
