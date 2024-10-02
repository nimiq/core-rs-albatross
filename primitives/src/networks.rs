use std::{
    fmt::{Display, Formatter},
    str::FromStr,
};

use thiserror::Error;

#[derive(Copy, Clone, Ord, PartialOrd, Eq, PartialEq, Debug, Hash)]
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

#[derive(Error, Debug)]
#[error("not a valid network ID")]
pub struct NetworkIdFromU8Error;

impl TryFrom<u8> for NetworkId {
    type Error = NetworkIdFromU8Error;
    fn try_from(value: u8) -> Result<NetworkId, NetworkIdFromU8Error> {
        use NetworkId::*;
        Ok(match value {
            1 => Test,
            2 => Dev,
            3 => Bounty,
            4 => Dummy,
            42 => Main,

            5 => TestAlbatross,
            6 => DevAlbatross,
            7 => UnitAlbatross,
            24 => MainAlbatross,

            _ => return Err(NetworkIdFromU8Error),
        })
    }
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
        match s {
            "Test" | "test" => Ok(NetworkId::Test),
            "Dev" | "dev" => Ok(NetworkId::Dev),
            "Bounty" | "bounty" => Ok(NetworkId::Bounty),
            "Dummy" | "dummy" => Ok(NetworkId::Dummy),
            "Main" | "main" => Ok(NetworkId::Main),
            "UnitAlbatross" | "unitalbatross" | "unit-albatross" => Ok(NetworkId::UnitAlbatross),
            "TestAlbatross" | "testalbatross" | "test-albatross" => Ok(NetworkId::TestAlbatross),
            "DevAlbatross" | "devalbatross" | "dev-albatross" => Ok(NetworkId::DevAlbatross),
            "MainAlbatross" | "mainalbatross" | "main-albatross" => Ok(NetworkId::MainAlbatross),
            _ => Err(NetworkIdParseError(String::from(s))),
        }
    }
}

#[derive(Error, Debug)]
#[error("Input is not a valid albatross network: {0}")]
pub struct NetworkIdNotAlbatross(String);

impl NetworkId {
    pub fn as_str(self) -> &'static str {
        match self {
            NetworkId::Test => "Test",
            NetworkId::Dev => "Dev",
            NetworkId::Bounty => "Bounty",
            NetworkId::Dummy => "Dummy",
            NetworkId::Main => "Main",
            NetworkId::TestAlbatross => "TestAlbatross",
            NetworkId::DevAlbatross => "DevAlbatross",
            NetworkId::UnitAlbatross => "UnitAlbatross",
            NetworkId::MainAlbatross => "MainAlbatross",
        }
    }

    /// This returns the default path for the zk keys based on the network id.
    /// Important note: Changing these constants must be reflected on several other places.
    pub fn default_zkp_path(&self) -> Result<&'static str, NetworkIdNotAlbatross> {
        match self {
            NetworkId::TestAlbatross => Ok(".zkp_testnet"),
            NetworkId::DevAlbatross => Ok(".zkp_devnet"),
            NetworkId::UnitAlbatross => Ok(".zkp_tests"),
            NetworkId::MainAlbatross => Ok(".zkp_mainnet"),
            _ => Err(NetworkIdNotAlbatross(self.as_str().to_owned())),
        }
    }
}

impl Display for NetworkId {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        self.as_str().fmt(f)
    }
}

#[cfg(feature = "serde-derive")]
mod serde_derive {
    use std::str::FromStr;

    use super::NetworkId;

    impl nimiq_serde::SerializedSize for NetworkId {
        const SIZE: usize = 1;
    }

    impl serde::Serialize for NetworkId {
        fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            if serializer.is_human_readable() {
                serializer.serialize_str(self.as_str())
            } else {
                serializer.serialize_u8(*self as u8)
            }
        }
    }

    impl<'de> serde::Deserialize<'de> for NetworkId {
        fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
            use serde::de::{Error as _, Unexpected};
            if deserializer.is_human_readable() {
                let intermediate = String::deserialize(deserializer)?;
                NetworkId::from_str(&intermediate).map_err(|_| {
                    D::Error::invalid_value(Unexpected::Str(&intermediate), &"a valid network name")
                })
            } else {
                // No need to fuzz, delegates to `u8` and `NetworkId::try_from`
                // looks sane.
                let intermediate = u8::deserialize(deserializer)?;
                NetworkId::try_from(intermediate).map_err(|_| {
                    D::Error::invalid_value(
                        Unexpected::Unsigned(intermediate.into()),
                        &"an ID corresponding to a network",
                    )
                })
            }
        }
    }
}
