use std::convert::TryFrom;
use std::sync::{Arc, Weak};

#[cfg(feature="validator")]
use validator::validator::Validator;
use consensus::{
    Consensus as AbstractConsensus,
    AlbatrossConsensusProtocol,
};
use database::Environment;
use network::{NetworkConfig, Network as GenericNetwork};
use mempool::Mempool as GenericMempool;
use network_primitives::services::ServiceFlags;
use blockchain::Blockchain;

use crate::error::Error;
use crate::config::config::{ClientConfig, ProtocolConfig};


/// Alias for the Consensus specialized over Albatross
pub type Consensus = AbstractConsensus<AlbatrossConsensusProtocol>;
pub type Mempool = GenericMempool<Blockchain>;
pub type Network = GenericNetwork<Blockchain>;


/// Holds references to the relevant structs. This is then Arc'd in `Client` and a nice API is
/// exposed.
///
/// # ToDos
///
/// * Add `WalletStore` here.
/// * Move RPC server, Ws-RPC server and Metrics server out of here
/// * Move Validator out of here?
///
pub(crate) struct ClientInner {
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


impl TryFrom<ClientConfig> for ClientInner {
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
            ProtocolConfig::Wss { host, port, tls_credentials } => {
                let key_file = tls_credentials.key_file.to_str()
                    .unwrap_or_else(|| panic!("Failed to convert path to PKCS#12 key file to string: {}", tls_credentials.key_file.display()))
                    .to_string();
                NetworkConfig::new_wss_network_config(host, port, false, key_file, tls_credentials.passphrase)
            }
        };

        // Set user agent
        network_config.set_user_agent(config.user_agent.into());

        // Set custom seeds
        network_config.set_additional_seeds(config.seeds);

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
        let environment = config.storage.database(config.network, config.consensus, config.database)?;

        // Create Nimiq consensus
        if !config.network.is_albatross() {
            return Err(Error::config_error(&format!("{} is not compatible with Albatross", config.network)));
        }
        let consensus = Consensus::new(
            environment.clone(),
            config.network,
            network_config,
            config.mempool,
        )?;

        #[cfg(feature="validator")]
        let validator = config.validator.map(|_config| {
            Validator::new(Arc::clone(&consensus), validator_key)
        }).transpose()?;

        Ok(ClientInner {
            environment,
            consensus,
            #[cfg(feature="validator")]
            validator,
        })
    }
}



/// Entry point for the Nimiq client API.
///
/// This client object abstracts a complete Nimiq client. Many internal objects are exposed:
///
/// * `Consensus` - Contains most other objects, such as blockchain, mempool, etc.
/// * `Blockchain` - The blockchain. Use this to query blocks or transactions
/// * `Validator` - If the client runs a validator, this exposes access to the validator state,
///     such as progress of current signature aggregations.
/// * `Database` - This can be stored to store arbitrary byte strings along-side the consensus state
///     (e.g. the chain info). Make sure you don't collide with database names - e.g. by prefixing
///     them with something.
/// * ...
///
/// # ToDo
///
/// * Shortcuts for common tasks, such at `get_block`.
/// * Register listeners for certain events.
///
pub struct Client {
    inner: Arc<ClientInner>
}


impl Client {
    /// Initializes the Nimiq network stack.
    pub async fn initialize(&self) -> Result<(), Error> {
        self.inner.consensus.network.initialize().await?;
        Ok(())
    }

    /// After calling this the network stack will start connecting to other peers.
    pub fn connect(&self) -> Result<(), Error> {
        self.inner.consensus.network.connect()?;
        Ok(())
    }

    /// Returns a reference to the *Consensus*.
    pub fn consensus(&self) -> Arc<Consensus> {
        Arc::clone(&self.inner.consensus)
    }

    /// Returns a reference to the *Network* stack
    pub fn network(&self) -> Arc<Network> {
        Arc::clone(&self.inner.consensus.network)
    }

    /// Returns a reference to the blockchain
    pub fn blockchain(&self) -> Arc<Blockchain> {
        Arc::clone(&self.inner.consensus.blockchain)
    }

    /// Returns a reference to the *Mempool*
    pub fn mempool(&self) -> Arc<Mempool> {
        Arc::clone(&self.inner.consensus.mempool)
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
    pub(crate) fn inner_weak(&self) -> Weak<ClientInner> {
        Arc::downgrade(&self.inner)
    }

    pub(crate) fn inner(&self) -> Arc<ClientInner> {
        Arc::clone(&self.inner)
    }
}

impl TryFrom<ClientConfig> for Client {
    type Error = Error;

    fn try_from(config: ClientConfig) -> Result<Self, Self::Error> {
        let inner = ClientInner::try_from(config)?;
        Ok(Client { inner: Arc::new(inner) })
    }
}

impl Clone for Client {
    fn clone(&self) -> Self {
        Client { inner: Arc::clone(&self.inner)}
    }
}
