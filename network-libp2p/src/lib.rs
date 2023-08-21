#[macro_use]
extern crate log;

mod behaviour;
mod config;
mod connection_pool;
pub mod discovery;
pub mod dispatch;
mod error;
mod network;
#[cfg(feature = "metrics")]
mod network_metrics;
mod rate_limiting;

pub const DISCOVERY_PROTOCOL: &str = "/nimiq/discovery/0.0.1";

pub use config::{Config, TlsConfig};
pub use error::NetworkError;
pub use libp2p::{
    self,
    identity::{ed25519::Keypair as Ed25519KeyPair, Keypair},
    swarm::NetworkInfo,
    PeerId,
};
pub use network::Network;
use serde::{
    de::Error, ser::Error as SerializationError, Deserialize, Deserializer, Serialize, Serializer,
};

/// Wrapper to libp2p Keypair identity that implements SerDe Serialize/Deserialize
#[derive(Clone, Debug)]
pub struct Libp2pKeyPair(pub libp2p::identity::Keypair);

impl Serialize for Libp2pKeyPair {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        if let Ok(pk) = self.0.clone().try_into_ed25519() {
            serde_big_array::BigArray::serialize(&pk.to_bytes(), serializer)
        } else {
            Err(S::Error::custom("Unsupported key type"))
        }
    }
}

impl<'de> Deserialize<'de> for Libp2pKeyPair {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let mut hex_encoded: [u8; 64] = serde_big_array::BigArray::deserialize(deserializer)?;

        let keypair = libp2p::identity::ed25519::Keypair::try_from_bytes(&mut hex_encoded)
            .map_err(|_| D::Error::custom("Invalid value"))?;

        Ok(Libp2pKeyPair(Keypair::from(keypair)))
    }
}
