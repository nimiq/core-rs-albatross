use parking_lot::{Mutex, RwLock};
#[cfg(feature = "zkp-prover")]
use rand::SeedableRng;
#[cfg(feature = "zkp-prover")]
use rand_chacha::ChaCha20Rng;
#[cfg(feature = "zkp-prover")]
use std::path::PathBuf;
use std::sync::Arc;

use nimiq_block::Block;
#[cfg(feature = "full-consensus")]
use nimiq_blockchain::{Blockchain, BlockchainConfig};
use nimiq_blockchain_interface::AbstractBlockchain;
use nimiq_blockchain_proxy::BlockchainProxy;
use nimiq_bls::cache::PublicKeyCache;
use nimiq_consensus::{
    sync::syncer_proxy::SyncerProxy, Consensus as AbstractConsensus,
    ConsensusProxy as AbstractConsensusProxy,
};
use nimiq_genesis::{NetworkId, NetworkInfo};
use nimiq_light_blockchain::LightBlockchain;
#[cfg(feature = "validator")]
use nimiq_mempool::mempool::Mempool;
use nimiq_network_interface::network::Network as NetworkInterface;
use nimiq_network_libp2p::{
    discovery::peer_contacts::{PeerContact, Services},
    Config as NetworkConfig, Multiaddr, Network, Protocol,
};
use nimiq_primitives::{policy::Policy, task_executor::TaskExecutor};
use nimiq_utils::time::OffsetTime;
#[cfg(feature = "validator")]
use nimiq_validator::validator::Validator as AbstractValidator;
#[cfg(feature = "validator")]
use nimiq_validator::validator::ValidatorProxy as AbstractValidatorProxy;
#[cfg(feature = "validator")]
use nimiq_validator_network::network_impl::ValidatorNetworkImpl;
#[cfg(feature = "wallet")]
use nimiq_wallet::WalletStore;
#[cfg(feature = "zkp-prover")]
use nimiq_zkp::ZKP_VERIFYING_KEY;
#[cfg(feature = "zkp-prover")]
use nimiq_zkp_circuits::{
    setup::{all_files_created, load_verifying_key_from_file, setup, DEVELOPMENT_SEED},
    DEFAULT_KEYS_PATH,
};
#[cfg(feature = "database-storage")]
use nimiq_zkp_component::proof_store::{DBProofStore, ProofStore};
use nimiq_zkp_component::zkp_component::{
    ZKPComponent as AbstractZKPComponent, ZKPComponentProxy as AbstractZKPComponentProxy,
};

use crate::config::config::{ClientConfig, SyncMode};
use crate::error::Error;

/// Alias for the Consensus and Validator specialized over libp2p network
pub type Consensus = AbstractConsensus<Network>;
pub type ConsensusProxy = AbstractConsensusProxy<Network>;
#[cfg(feature = "validator")]
pub type Validator = AbstractValidator<Network, ValidatorNetworkImpl<Network>>;
#[cfg(feature = "validator")]
pub type ValidatorProxy = AbstractValidatorProxy;

pub type ZKPComponent = AbstractZKPComponent<Network>;
pub type ZKPComponentProxy = AbstractZKPComponentProxy<Network>;

/// Holds references to the relevant structs. This is then Arc'd in `Client` and a nice API is
/// exposed.
///
/// # TODO
///
/// * Move RPC server and Metrics server out of here
/// * Move Validator out of here?
///
pub(crate) struct ClientInner {
    network: Arc<Network>,

    /// The consensus object, which maintains the blockchain, the network and other things to
    /// reach consensus.
    consensus: ConsensusProxy,

    blockchain: BlockchainProxy,

    #[cfg(feature = "validator")]
    validator: Option<ValidatorProxy>,

    /// Wallet that stores key pairs for transaction signing
    #[cfg(feature = "wallet")]
    wallet_store: Arc<WalletStore>,

    zkp_component: ZKPComponentProxy,
}

/// This function is used to generate the services flags (provided, needed) based upon the configured sync mode
pub fn generate_service_flags(sync_mode: SyncMode) -> (Services, Services) {
    let provided_services = match sync_mode {
        // Services provided by history nodes
        crate::config::config::SyncMode::History => {
            log::info!("Client configured as a history node");
            Services::HISTORY
                | Services::FULL_BLOCKS
                | Services::ACCOUNTS_PROOF
                | Services::ACCOUNTS_CHUNKS
                | Services::TRANSACTION_INDEX
        }
        // Services provided by full nodes
        crate::config::config::SyncMode::Full => {
            log::info!("Client configured as a full node");
            Services::ACCOUNTS_PROOF | Services::FULL_BLOCKS | Services::ACCOUNTS_CHUNKS
        }
        // Services provided by light nodes
        crate::config::config::SyncMode::Light => {
            log::info!("Client configured as a light node");
            Services::empty()
        }
    };

    let required_services = match sync_mode {
        // Services required by history nodes
        crate::config::config::SyncMode::History => Services::HISTORY | Services::FULL_BLOCKS,
        // Services required by full nodes
        crate::config::config::SyncMode::Full => Services::FULL_BLOCKS | Services::ACCOUNTS_CHUNKS,
        // Services required by light nodes
        crate::config::config::SyncMode::Light => Services::ACCOUNTS_PROOF,
    };
    (provided_services, required_services)
}

impl ClientInner {
    async fn from_config(
        config: ClientConfig,
        executor: impl TaskExecutor + Send + 'static + Clone,
    ) -> Result<Client, Error> {
        // Get network info (i.e. which specific blockchain we're on)
        if !config.network_id.is_albatross() {
            return Err(Error::config_error(format!(
                "{} is not compatible with Albatross",
                config.network_id
            )));
        }
        let network_info = NetworkInfo::from_network_id(config.network_id);

        #[cfg(not(feature = "zkp-prover"))]
        if config.zkp.prover_active {
            panic!("Can't build a prover node without the zkp-prover feature enabled")
        }

        #[cfg(feature = "zkp-prover")]
        // If the Prover is active for devnet we need to ensure that the proving keys are present.
        if config.network_id == NetworkId::DevAlbatross
            && config.zkp.prover_active
            && !all_files_created(&config.zkp.prover_keys_path, config.zkp.prover_active)
        {
            log::info!("Setting up zero-knowledge prover keys for devnet.");
            log::info!("This task only needs to be run once and might take about an hour.");
            log::info!(
                "Alternatively, you can place the proving keys in this folder: {:?}.",
                config.zkp.prover_keys_path
            );
            setup(
                ChaCha20Rng::from_seed(DEVELOPMENT_SEED),
                &PathBuf::from(DEFAULT_KEYS_PATH),
                config.zkp.prover_active,
            )?;
            log::info!("Setting the verification key.");
            let vk = load_verifying_key_from_file(&config.zkp.prover_keys_path)?;
            assert_eq!(vk, *ZKP_VERIFYING_KEY, "Verifying keys don't match. The build in verifying keys don't match the newly generated ones.");
            log::debug!("Finished ZKP setup.");
        }

        // Initialize clock
        let time = Arc::new(OffsetTime::new());

        // Load identity keypair from file store
        let identity_keypair = config.storage.identity_keypair()?;
        log::info!("Identity public key: {:?}", identity_keypair.public());
        log::info!(
            "PeerId: {:}",
            identity_keypair.public().to_peer_id().to_base58()
        );

        let (mut provided_services, required_services) =
            generate_service_flags(config.consensus.sync_mode);

        // We update the services flags depending on our validator configuration
        #[cfg(feature = "validator")]
        if config.validator.is_some() {
            provided_services |= Services::VALIDATOR;
        }

        // Generate my peer contact from identity keypair and my provided services
        // Filter out unspecified IP addresses since those are not addresses suitable
        // for the contact book (for others to contact ourself).
        let mut peer_contact_addresses = config.network.listen_addresses.clone();
        peer_contact_addresses.retain(|address| {
            let mut protocols = address.iter();
            match protocols.next() {
                Some(Protocol::Ip4(ip)) => !ip.is_unspecified(),
                Some(Protocol::Ip6(ip)) => !ip.is_unspecified(),
                _ => true,
            }
        });
        let mut peer_contact = PeerContact::new(
            peer_contact_addresses,
            identity_keypair.public(),
            provided_services,
            None,
        );
        peer_contact.set_current_time();

        let seeds: Vec<Multiaddr> = config
            .network
            .seeds
            .clone()
            .into_iter()
            .map(|seed| seed.address)
            .collect();

        // Setup libp2p network
        let network_config = NetworkConfig::new(
            identity_keypair,
            peer_contact,
            seeds,
            network_info.genesis_hash().clone(),
            false,
            required_services,
        );

        log::debug!("listen_addresses = {:?}", config.network.listen_addresses);

        let network =
            Arc::new(Network::new(Arc::clone(&time), network_config, executor.clone()).await);

        // Start buffering network events as early as possible
        let network_events = network.subscribe_events();

        // Open database
        #[cfg(feature = "database-storage")]
        let environment = config.storage.database(
            config.network_id,
            config.consensus.sync_mode,
            config.database,
        )?;

        let bls_cache = Arc::new(Mutex::new(PublicKeyCache::new(
            Policy::BLS_CACHE_MAX_CAPACITY,
        )));

        #[cfg(feature = "full-consensus")]
        let mut blockchain_config = BlockchainConfig {
            max_epochs_stored: config.consensus.max_epochs_stored,
            ..Default::default()
        };

        #[cfg(feature = "database-storage")]
        let zkp_storage: Option<Box<dyn ProofStore>> =
            Some(Box::new(DBProofStore::new(environment.clone())));
        #[cfg(not(feature = "database-storage"))]
        let zkp_storage = None;

        let (blockchain_proxy, syncer_proxy, zkp_component) = match config.consensus.sync_mode {
            #[cfg(not(feature = "full-consensus"))]
            SyncMode::History => {
                panic!("Can't build a history node without the full-consensus feature enabled")
            }
            #[cfg(not(feature = "full-consensus"))]
            SyncMode::Full => {
                panic!("Can't build a full node without the full-consensus feature enabled")
            }
            #[cfg(feature = "full-consensus")]
            SyncMode::History => {
                blockchain_config.keep_history = true;
                let blockchain = Arc::new(RwLock::new(
                    Blockchain::new(
                        environment.clone(),
                        blockchain_config,
                        config.network_id,
                        time,
                    )
                    .unwrap(),
                ));
                let blockchain_proxy = BlockchainProxy::from(&blockchain);
                #[cfg(feature = "zkp-prover")]
                let zkp_component = if config.zkp.prover_active {
                    ZKPComponent::with_prover(
                        blockchain_proxy.clone(),
                        Arc::clone(&network),
                        executor.clone(),
                        config.zkp.prover_active,
                        None,
                        config.zkp.prover_keys_path,
                        zkp_storage,
                    )
                    .await
                } else {
                    ZKPComponent::new(
                        blockchain_proxy.clone(),
                        Arc::clone(&network),
                        executor.clone(),
                        zkp_storage,
                    )
                    .await
                };
                #[cfg(not(feature = "zkp-prover"))]
                let zkp_component = ZKPComponent::new(
                    blockchain_proxy.clone(),
                    Arc::clone(&network),
                    executor.clone(),
                    zkp_storage,
                )
                .await;
                let syncer = SyncerProxy::new_history(
                    blockchain_proxy.clone(),
                    Arc::clone(&network),
                    bls_cache,
                    network_events,
                )
                .await;
                (blockchain_proxy, syncer, zkp_component)
            }
            #[cfg(feature = "full-consensus")]
            SyncMode::Full => {
                blockchain_config.keep_history = false;

                let blockchain = Arc::new(RwLock::new(
                    Blockchain::new(
                        environment.clone(),
                        blockchain_config,
                        config.network_id,
                        time,
                    )
                    .unwrap(),
                ));
                let blockchain_proxy = BlockchainProxy::from(&blockchain);
                #[cfg(feature = "zkp-prover")]
                let zkp_component = if config.zkp.prover_active {
                    ZKPComponent::with_prover(
                        blockchain_proxy.clone(),
                        Arc::clone(&network),
                        executor.clone(),
                        config.zkp.prover_active,
                        None,
                        config.zkp.prover_keys_path,
                        zkp_storage,
                    )
                    .await
                } else {
                    ZKPComponent::new(
                        blockchain_proxy.clone(),
                        Arc::clone(&network),
                        executor.clone(),
                        zkp_storage,
                    )
                    .await
                };
                #[cfg(not(feature = "zkp-prover"))]
                let zkp_component = ZKPComponent::new(
                    blockchain_proxy.clone(),
                    Arc::clone(&network),
                    executor.clone(),
                    zkp_storage,
                )
                .await;

                let syncer = SyncerProxy::new_full(
                    blockchain_proxy.clone(),
                    Arc::clone(&network),
                    bls_cache,
                    zkp_component.proxy(),
                    network_events,
                )
                .await;
                (blockchain_proxy, syncer, zkp_component)
            }
            SyncMode::Light => {
                let blockchain = Arc::new(RwLock::new(LightBlockchain::new(config.network_id)));
                let blockchain_proxy = BlockchainProxy::from(&blockchain);
                let zkp_component = ZKPComponent::new(
                    blockchain_proxy.clone(),
                    Arc::clone(&network),
                    executor.clone(),
                    zkp_storage,
                )
                .await;
                let syncer = SyncerProxy::new_light(
                    blockchain_proxy.clone(),
                    Arc::clone(&network),
                    bls_cache,
                    zkp_component.proxy(),
                    network_events,
                    executor.clone(),
                )
                .await;
                (blockchain_proxy, syncer, zkp_component)
            }
        };

        // Open wallet
        #[cfg(feature = "wallet")]
        let wallet_store = Arc::new(WalletStore::new(environment.clone()));

        // Initialize consensus
        let consensus = Consensus::new(
            blockchain_proxy.clone(),
            Arc::clone(&network),
            syncer_proxy,
            config.consensus.min_peers,
            zkp_component.proxy(),
            executor,
        );

        #[cfg(feature = "validator")]
        let (validator, validator_proxy) = match config.validator {
            Some(validator_config) => {
                if let BlockchainProxy::Full(ref blockchain) = blockchain_proxy {
                    // Load validator address
                    let validator_address = validator_config.validator_address;

                    // Load validator address
                    let automatic_reactivate = validator_config.automatic_reactivate;

                    // Load signing key (before we give away ownership of the storage config)
                    let signing_key = config.storage.signing_keypair()?;

                    // Load validator key (before we give away ownership of the storage config)
                    let voting_key = config.storage.voting_keypair()?;

                    // Load fee key (before we give away ownership of the storage config)
                    let fee_key = config.storage.fee_keypair()?;

                    let validator_network =
                        Arc::new(ValidatorNetworkImpl::new(Arc::clone(&network)));

                    let validator = Validator::new(
                        environment.clone(),
                        &consensus,
                        Arc::clone(blockchain),
                        validator_network,
                        validator_address,
                        automatic_reactivate,
                        signing_key,
                        voting_key,
                        fee_key,
                        config.mempool,
                    );

                    // Use the validator's mempool as TransactionVerificationCache in the blockchain.
                    blockchain.write().tx_verification_cache =
                        Arc::<Mempool>::clone(&validator.mempool);

                    let validator_proxy = validator.proxy();
                    (Some(validator), Some(validator_proxy))
                } else {
                    (None, None)
                }
            }
            None => (None, None),
        };

        // Start network.
        network.listen_on(config.network.listen_addresses).await;
        network.start_connecting().await;

        Ok(Client {
            inner: Arc::new(ClientInner {
                network,
                consensus: consensus.proxy(),
                blockchain: blockchain_proxy,
                #[cfg(feature = "validator")]
                validator: validator_proxy,
                #[cfg(feature = "wallet")]
                wallet_store,
                zkp_component: zkp_component.proxy(),
            }),
            consensus: Some(consensus),
            #[cfg(feature = "validator")]
            validator,
            zkp_component: Some(zkp_component),
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
    consensus: Option<Consensus>,
    #[cfg(feature = "validator")]
    validator: Option<Validator>,
    zkp_component: Option<ZKPComponent>,
}

impl Client {
    pub async fn from_config(
        config: ClientConfig,
        executor: impl TaskExecutor + Send + 'static + Clone,
    ) -> Result<Self, Error> {
        ClientInner::from_config(config, executor).await
    }

    pub fn take_consensus(&mut self) -> Option<Consensus> {
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
    pub fn blockchain(&self) -> BlockchainProxy {
        self.inner.blockchain.clone()
    }

    /// Returns the blockchain head
    pub fn blockchain_head(&self) -> Block {
        self.inner.blockchain.read().head()
    }

    #[cfg(feature = "wallet")]
    pub fn wallet_store(&self) -> Arc<WalletStore> {
        Arc::clone(&self.inner.wallet_store)
    }

    /// Returns a reference to the *Validator* or `None`.
    #[cfg(feature = "validator")]
    pub fn take_validator(&mut self) -> Option<Validator> {
        self.validator.take()
    }

    #[cfg(feature = "validator")]
    /// Returns a reference to the *Validator proxy*.
    pub fn validator_proxy(&self) -> Option<ValidatorProxy> {
        self.inner.validator.clone()
    }

    #[cfg(feature = "validator")]
    pub fn mempool(&self) -> Option<Arc<Mempool>> {
        self.validator
            .as_ref()
            .map(|validator| Arc::clone(&validator.mempool))
    }

    /// Returns a reference to the *ZKP Component* or none.
    pub fn take_zkp_component(&mut self) -> Option<ZKPComponent> {
        self.zkp_component.take()
    }

    /// Returns a reference to the *ZKP Component Proxy*.
    pub fn zkp_component(&self) -> ZKPComponentProxy {
        self.inner.zkp_component.clone()
    }
}
