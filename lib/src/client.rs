use std::sync::Arc;

use nimiq_block_albatross::Block;
use nimiq_blockchain_albatross::{AbstractBlockchain, Blockchain};
use nimiq_consensus_albatross::{
    Consensus as AbstractConsensus, ConsensusProxy as AbstractConsensusProxy,
};
use nimiq_database::Environment;
use nimiq_genesis::NetworkInfo;
use nimiq_mempool::Mempool;
use nimiq_network_interface::network::Network as NetworkInterface;
use nimiq_network_libp2p::{
    discovery::peer_contacts::{PeerContact, Services},
    Config as NetworkConfig, Network,
};
use nimiq_utils::time::OffsetTime;

#[cfg(feature = "validator")]
use nimiq_validator::validator::Validator as AbstractValidator;
#[cfg(feature = "validator")]
use nimiq_validator_network::network_impl::ValidatorNetworkImpl;
#[cfg(feature = "wallet")]
use nimiq_wallet::WalletStore;

use crate::config::config::ClientConfig;
use crate::error::Error;
use nimiq_consensus_albatross::sync::history::HistorySync;
use nimiq_network_libp2p::libp2p::futures::StreamExt;

/// Alias for the Consensus and Validator specialized over libp2p network
pub type Consensus = AbstractConsensus<Network>;
pub type ConsensusProxy = AbstractConsensusProxy<Network>;
pub type Validator = AbstractValidator<Network, ValidatorNetworkImpl<Network>>;

/// Holds references to the relevant structs. This is then Arc'd in `Client` and a nice API is
/// exposed.
///
/// # TODO
///
/// * Move RPC server, Ws-RPC server and Metrics server out of here
/// * Move Validator out of here?
///
pub(crate) struct ClientInner {
    /// The database environment. This is here to give the consumer access to the DB too. This
    /// reference is also stored in the consensus though.
    environment: Environment,

    network: Arc<Network>,

    /// The consensus object, which maintains the blockchain, the network and other things to
    /// reach consensus.
    consensus: ConsensusProxy,

    /// Wallet that stores keypairs for transaction signing
    #[cfg(feature = "wallet")]
    wallet_store: Arc<WalletStore>,
}

impl ClientInner {
    async fn from_config(
        config: ClientConfig,
    ) -> Result<(Self, Consensus, Option<Validator>), Error> {
        // Get network info (i.e. which specific blokchain we're on)
        if !config.network_id.is_albatross() {
            return Err(Error::config_error(&format!(
                "{} is not compatible with Albatross",
                config.network_id
            )));
        }
        let network_info = NetworkInfo::from_network_id(config.network_id);

        // Initialize clock
        let time = Arc::new(OffsetTime::new());

        // Load identity keypair from file store
        let identity_keypair = config.storage.identity_keypair()?;
        log::info!("Identity public key: {:?}", identity_keypair.public());

        // Generate peer contact from identity keypair and services/protocols
        let mut peer_contact = PeerContact::new(
            config.network.listen_addresses.clone(),
            identity_keypair.public(),
            Services::all(), // TODO
            None,
        );
        peer_contact.set_current_time();

        // Setup libp2p network
        let mut network_config = NetworkConfig::new(
            identity_keypair,
            peer_contact,
            network_info.genesis_hash().clone(),
        );
        if let Some(min_peers) = config.network.min_peers {
            network_config.min_peers = min_peers;
        }

        log::debug!("listen_addresses = {:?}", config.network.listen_addresses);

        let network = Arc::new(Network::new(Arc::clone(&time), network_config).await);

        // Start buffering network events as early as possible
        let nw_events = network.subscribe_events();

        // Load validator key (before we give away ownership of the storage config)
        #[cfg(feature = "validator")]
        let validator_key = config.storage.validator_key()?;

        // Open database
        let environment =
            config
                .storage
                .database(config.network_id, config.consensus, config.database)?;
        let blockchain = Arc::new(Blockchain::new(environment.clone(), config.network_id).unwrap());
        let mempool = Mempool::new(Arc::clone(&blockchain), config.mempool);

        // Open wallet
        #[cfg(feature = "wallet")]
        let wallet_store = Arc::new(WalletStore::new(environment.clone()));

        let sync = HistorySync::<Network>::new(Arc::clone(&blockchain), nw_events);

        let consensus = Consensus::from_network(
            environment.clone(),
            blockchain,
            mempool,
            Arc::clone(&network),
            sync.boxed(),
        )
        .await;

        // Tell the network to connect to seed nodes
        for seed in &config.network.seeds {
            log::debug!("Dialing seed: {:?}", seed);
            network.dial_address(seed.address.clone()).await?;
        }
        // tell the network to listen on the given addresses
        network
            .listen_on_addresses(config.network.listen_addresses)
            .await;

        #[cfg(feature = "validator")]
        let validator = {
            if let Some(config) = &config.validator {
                #[cfg(not(feature = "wallet"))]
                let validator_wallet_key = {
                    log::warn!("Client is compiled without wallet and thus can't load the wallet account for the validator.");
                    None
                };

                #[cfg(feature = "wallet")]
                let validator_wallet_key = {
                    if let Some(wallet_account) = &config.wallet_account {
                        let address = wallet_account.parse().map_err(|_| {
                            Error::config_error(format!(
                                "Failed to parse validator wallet address: {}",
                                wallet_account
                            ))
                        })?;
                        let locked = wallet_store.get(&address, None).ok_or_else(|| {
                            Error::config_error(format!(
                                "Could not find wallet account: {}",
                                wallet_account
                            ))
                        })?;
                        let unlocked = locked
                            .unlock(
                                config
                                    .wallet_password
                                    .clone()
                                    .unwrap_or_default()
                                    .as_bytes(),
                            )
                            .map_err(|_| {
                                Error::config_error(format!(
                                    "Failed to unlock validator wallet account: {}",
                                    wallet_account
                                ))
                            })?;
                        Some(unlocked.key_pair.clone())
                    } else {
                        None
                    }
                };

                let validator_network = Arc::new(ValidatorNetworkImpl::new(Arc::clone(&network)));

                let validator = Validator::new(
                    &consensus,
                    validator_network,
                    validator_key,
                    validator_wallet_key,
                );

                Some(validator)
            } else {
                None
            }
        };

        Ok((
            ClientInner {
                environment,
                network,
                consensus: consensus.proxy(),
                #[cfg(feature = "wallet")]
                wallet_store,
            },
            consensus,
            validator,
        ))
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
    inner: Arc<ClientInner>,
    consensus: Option<Consensus>,
    validator: Option<Validator>,
}

impl Client {
    pub async fn from_config(config: ClientConfig) -> Result<Self, Error> {
        let (inner, consensus, validator) = ClientInner::from_config(config).await?;
        Ok(Client {
            inner: Arc::new(inner),
            consensus: Some(consensus),
            validator,
        })
    }

    pub fn consensus(&mut self) -> Option<Consensus> {
        self.consensus.take()
    }

    /// Returns a reference to the *Consensus proxy*.
    pub fn consensus_proxy(&self) -> ConsensusProxy {
        self.inner.consensus.clone()
    }

    /// Returns a reference to the *Network* stack
    pub fn network(&self) -> Arc<Network> {
        Arc::clone(&self.inner.network)
    }

    /// Returns a reference to the blockchain
    pub fn blockchain(&self) -> Arc<Blockchain> {
        Arc::clone(&self.inner.consensus.blockchain)
    }

    /// Returns the blockchain head
    pub fn blockchain_head(&self) -> Block {
        Arc::clone(&self.inner.consensus.blockchain).head()
    }

    /// Returns a reference to the *Mempool*
    pub fn mempool(&self) -> Arc<Mempool> {
        Arc::clone(&self.inner.consensus.mempool)
    }

    #[cfg(feature = "wallet")]
    pub fn wallet_store(&self) -> Arc<WalletStore> {
        Arc::clone(&self.inner.wallet_store)
    }

    /// Returns a reference to the *Validator* or `None`.
    #[cfg(feature = "validator")]
    pub fn validator(&mut self) -> Option<Validator> {
        self.validator.take()
    }

    /// Returns the database environment.
    pub fn environment(&self) -> Environment {
        self.inner.environment.clone()
    }
}
