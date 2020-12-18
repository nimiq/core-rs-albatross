use std::{convert::TryFrom, sync::Arc};

use nimiq_blockchain_albatross::Blockchain;
use nimiq_consensus_albatross::{sync::QuickSync, Consensus as AbstractConsensus};
use nimiq_database::Environment;
use nimiq_genesis::NetworkInfo;
use nimiq_mempool::Mempool;
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

/// Alias for the Consensus and Validator specialized over libp2p network
pub type Consensus = AbstractConsensus<Network>;
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

    /// The consensus object, which maintains the blockchain, the network and other things to
    /// reach consensus.
    consensus: Arc<Consensus>,

    /*
    /// The block production logic. This is optional and can also be fully disabled at compile-time
    #[cfg(feature = "validator")]
    validator: Option<Arc<Validator>>,
    */
    /// Wallet that stores keypairs for transaction signing
    #[cfg(feature = "wallet")]
    wallet_store: WalletStore,
}

impl TryFrom<ClientConfig> for ClientInner {
    type Error = Error;

    fn try_from(config: ClientConfig) -> Result<Self, Self::Error> {
        // Get network info (i.e. which specific blokchain we're on)
        if !config.network_id.is_albatross() {
            return Err(Error::config_error(&format!("{} is not compatible with Albatross", config.network_id)));
        }
        let network_info = NetworkInfo::from_network_id(config.network_id);

        // Initialize clock
        let time = Arc::new(OffsetTime::new());

        // Load identity keypair from file store
        let identity_keypair = config.storage.identity_keypair()?;
        log::info!("Identity public key: {:?}", identity_keypair.public());

        // Generate peer contact from identity keypair and services/protocols
        let peer_contact = PeerContact::new(
            config.network.listen_addresses.clone(),
            identity_keypair.public(),
            Services::all(), // TODO
            None,            // No need to set the timestamp as this will be set before signing anyway.
        );

        // Setup libp2p network
        let network_config = NetworkConfig::new(identity_keypair, peer_contact, network_info.genesis_hash().clone());
        let network = Arc::new(Network::new(config.network.listen_addresses, Arc::clone(&time), network_config));

        // Load validator key (before we give away ownership of the storage config)
        #[cfg(feature = "validator")]
        let _validator_key = config.storage.validator_key();

        // Open database
        let environment = config.storage.database(config.network_id, config.consensus, config.database)?;
        let blockchain = Arc::new(Blockchain::new(environment.clone(), config.network_id).unwrap());
        let mempool = Mempool::new(Arc::clone(&blockchain), config.mempool);

        // Open wallet
        #[cfg(feature = "wallet")]
        let wallet_store = WalletStore::new(env);

        // TODO: This will need to be changed from the QuickSync protocol to a more adequate sync
        //       protocol.
        let sync = QuickSync::default();

        let consensus = Consensus::new(environment.clone(), blockchain, mempool, Arc::clone(&network), sync)?;

        /*
        #[cfg(feature = "validator")]
        let validator = {
            let validator_wallet_key = config.validator.map(|c| {
                #[cfg(not(feature = "wallet"))]
                {
                    // TODO: Maybe this should fail
                    log::error!("Client is compiled without wallet and thus can't load the wallet account for the validator.");
                    None
                }
                #[cfg(feature = "wallet")]
                Some(wallet_store.get(&c.wallet_account, None).unlock()?.key_pair)
            });

            let validator_network = ValidatorNetworkImpl::new(network);

            config
                .validator
                .map(|_config| Arc::new(Validator::new(Arc::clone(&consensus), validator_network, validator_key, validator_wallet_key)));
        };
        */

        Ok(ClientInner {
            environment,
            consensus,
            //#[cfg(feature = "validator")]
            //validator,
            #[cfg(feature = "wallet")]
            wallet_store,
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
    inner: Arc<ClientInner>,
}

impl Client {
    /// After calling this the network stack will start connecting to other peers.
    pub fn connect(&self) -> Result<(), Error> {
        // Tell the network to connect to seed nodes
        todo!()
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

    /*
    /// Returns a reference to the *Validator* or `None`.
    #[cfg(feature = "validator")]
    pub fn validator(&self) -> Option<Arc<Validator>> {
        self.inner.validator.as_ref().map(|v| Arc::clone(v))
    }
    */

    /// Returns the database environment.
    pub fn environment(&self) -> Environment {
        self.inner.environment.clone()
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
        Client {
            inner: Arc::clone(&self.inner),
        }
    }
}
