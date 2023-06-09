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

pub const REQRES_PROTOCOL: &[u8] = b"/nimiq/reqres/0.0.1";
pub const MESSAGE_PROTOCOL: &[u8] = b"/nimiq/message/0.0.1";
pub const DISCOVERY_PROTOCOL: &[u8] = b"/nimiq/discovery/0.0.1";

pub use config::{Config, TlsConfig};
pub use error::NetworkError;
pub use libp2p::{
    self,
    identity::{ed25519::Keypair as Ed25519KeyPair, Keypair},
    swarm::NetworkInfo,
    PeerId,
};
pub use network::Network;
use serde::{de::Error, Deserialize, Deserializer, Serialize, Serializer};

/// Wrapper to libp2p Keypair indetity that implements SerDe Serialize/Deserialize
#[derive(Clone, Debug)]
pub struct Libp2pKeyPair(pub libp2p::identity::Keypair);

impl Serialize for Libp2pKeyPair {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match &self.0 {
            Keypair::Ed25519(keypair) => {
                serde_big_array::BigArray::serialize(&keypair.encode(), serializer)
            }
        }
    }
}

impl<'de> Deserialize<'de> for Libp2pKeyPair {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let mut hex_encoded: [u8; 64] = serde_big_array::BigArray::deserialize(deserializer)?;

        let pk = libp2p::identity::ed25519::Keypair::decode(&mut hex_encoded)
            .map_err(|_| D::Error::custom("Invalid value"))?;
        Ok(Libp2pKeyPair(Keypair::Ed25519(pk)))
    }
}
