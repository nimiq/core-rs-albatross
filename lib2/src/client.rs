use std::convert::TryFrom;
use std::sync::{Arc, Weak};

#[cfg(feature="validator")]
use validator::validator::Validator;
use consensus::{
    Consensus as AbstractConsensus,
    AlbatrossConsensusProtocol,
};
use database::Environment;
use network::NetworkConfig;
use network_primitives::services::ServiceFlags;

use crate::error::Error;
use crate::config::config::{ClientConfig, ProtocolConfig};


/// Alias for the Consensus specialized over Albatross
pub type Consensus = AbstractConsensus<AlbatrossConsensusProtocol>;


/// Holds references to the relevant structs. This is then Arc'd in `Client` and a nice API is
/// exposed.
///
/// # ToDos
///
/// * Add `WalletStore` here.
struct _Client {
    /// The database environment. This is here to give the consumer access to the DB too. This
    /// reference is also stored in the consensus though.
    environment: Environment,

    /// The consensus object, which maintains the blockchain, the network and other things to
    /// reach consensus.
    consensus: Arc<Consensus>,

    /// The block production logic. This is optional and can also be fully disabled at compile-time
    #[cfg(feature="validator")]
    validator: Option<Arc<Validator>>
}


impl TryFrom<ClientConfig> for _Client {
    type Error = Error;

    fn try_from(config: ClientConfig) -> Result<Self, Self::Error> {
        // Create network config
        // TODO: `NetworkConfig` could use some refactoring. So we might as well adapt it to the
        // client API.

        let mut network_config = match config.protocol {
            ProtocolConfig::Dumb => {
                NetworkConfig::new_dumb_network_config()
            },
            ProtocolConfig::Rtc => {
                panic!("WebRTC is not yet implemented")
            },
            ProtocolConfig::Ws { host, port } => {
                NetworkConfig::new_ws_network_config(host, port, false, config.reverse_proxy)
            },
            ProtocolConfig::Wss { host, port, pkcs12_key_file, pkcs12_passphrase } => {
                let pkcs12_key_file = pkcs12_key_file.to_str()
                    .unwrap_or_else(|| panic!("Failed to convert path to PKCS#12 key file to string: {}", pkcs12_key_file.display()))
                    .to_string();
                NetworkConfig::new_wss_network_config(host, port, false, pkcs12_key_file, pkcs12_passphrase)
            }
        };

        // Set user agent
        network_config.set_user_agent(config.user_agent.into());

        // Initialize peer key
        config.storage.init_key_store(&mut network_config)?;

        // Load validator key (before we give away ownership of the storage config
        #[cfg(feature="validator")]
        let validator_key = config.storage.validator_key()
            .expect("Failed to load validator key");

        // Add validator service flag, if necessary
        #[cfg(feature="validator")]
        {
            if config.validator.is_some() {
                let mut services = network_config.services().clone();
                services.accepted |= ServiceFlags::VALIDATOR;
                services.provided |= ServiceFlags::VALIDATOR;
                network_config.set_services(services);
            }
        }

        // Open database
        let environment = config.storage.database(config.network_id, config.consensus)?;

        // Create Nimiq consensus
        let consensus = Consensus::new(
            environment.clone(),
            config.network_id,
            network_config,
            config.mempool,
        )?;

        #[cfg(feature="validator")]
        let validator = config.validator.map(|_config| {
            Validator::new(Arc::clone(&consensus), validator_key)
        }).transpose()?;

        Ok(_Client {
            environment,
            consensus,
            #[cfg(feature="validator")]
            validator,
        })
    }
}



/// Entry point for the Nimiq client API
pub struct Client {
    inner: Arc<_Client>
}


impl Client {
    /// Returns a reference to the *Consensus*.
    pub fn consensus(&self) -> Arc<Consensus> {
        Arc::clone(&self.inner.consensus)
    }

    /// Returns a reference to the *Validator* or `None`.
    #[cfg(feature="validator")]
    pub fn validator(&self) -> Option<Arc<Validator>> {
        self.inner.validator.as_ref().map(|v| Arc::clone(v))
    }

    /// Returns the database environment.
    pub fn environment(&self) -> Environment {
        self.inner.environment.clone()
    }

    /// Short-cut to get weak reference to the inner client object.
    /// TODO: We'll use this to register listeners
    fn weak(&self) -> Weak<_Client> {
        Arc::downgrade(&self.inner)
    }
}

impl TryFrom<ClientConfig> for Client {
    type Error = Error;

    fn try_from(config: ClientConfig) -> Result<Self, Self::Error> {
        let inner = _Client::try_from(config)?;
        Ok(Client { inner: Arc::new(inner) })
    }
}

impl Clone for Client {
    fn clone(&self) -> Self {
        Client { inner: Arc::clone(&self.inner)}
    }
}
