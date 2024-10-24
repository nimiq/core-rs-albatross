use std::{
    cell::{Cell, RefCell},
    collections::{
        hash_map::{Entry, HashMap},
        HashSet,
    },
    rc::Rc,
    str::FromStr,
};

use futures::{
    future::{select, Either},
    StreamExt,
};
use js_sys::{global, Array, Function, JsString};
use log::level_filters::LevelFilter;
use nimiq::client::ConsensusProxy;
pub use nimiq::{
    config::{
        config::ClientConfig,
        config_file::{LogSettings, Seed},
    },
    extras::{panic::initialize_panic_reporting, web_logging::initialize_web_logging},
};
use nimiq_blockchain_interface::{AbstractBlockchain, BlockchainEvent};
use nimiq_consensus::ConsensusEvent;
use nimiq_hash::Blake2bHash;
use nimiq_network_interface::{
    network::{CloseReason, Network, NetworkEvent},
    Multiaddr,
};
use nimiq_primitives::policy::Policy;
use nimiq_utils::spawn_local;
use tokio::sync::oneshot;
use tsify::Tsify;
use wasm_bindgen::{prelude::*, JsCast};
use web_sys::MessageEvent;

use crate::{
    client::{
        account::{
            PlainAccount, PlainAccountArrayType, PlainAccountType, PlainStaker,
            PlainStakerArrayType, PlainStakerType, PlainValidator, PlainValidatorArrayType,
            PlainValidatorType,
        },
        block::{PlainBlock, PlainBlockType},
        peer_info::{PlainPeerInfo, PlainPeerInfoArrayType},
    },
    common::{
        address::{Address, AddressAnyArrayType, AddressAnyType},
        client_configuration::{
            ClientConfiguration, PlainClientConfiguration, PlainClientConfigurationType,
        },
        transaction::{
            PlainTransactionDetails, PlainTransactionDetailsArrayType, PlainTransactionDetailsType,
            PlainTransactionReceipt, PlainTransactionReceiptArrayType,
            PlainTransactionRecipientData, Transaction, TransactionAnyType, TransactionState,
        },
        utils::from_network_id,
    },
};

/// Maximum number of transactions that can be requested by address
pub const MAX_TRANSACTIONS_BY_ADDRESS: u16 = 500;

/// Describes the state of consensus of the client.
#[derive(Tsify)]
#[serde(rename_all = "lowercase")]
pub enum ConsensusState {
    Connecting,
    Syncing,
    Established,
}

impl ConsensusState {
    pub fn to_string(&self) -> &str {
        match self {
            ConsensusState::Connecting => "connecting",
            ConsensusState::Syncing => "syncing",
            ConsensusState::Established => "established",
        }
    }
}

/// Nimiq Albatross client that runs in browsers via WASM and is exposed to Javascript.
///
/// ### Usage:
///
/// ```js
/// import init, * as Nimiq from "./pkg/nimiq_web_client.js";
///
/// init().then(async () => {
///     const config = new Nimiq.ClientConfiguration();
///     const client = await config.instantiateClient();
///     // ...
/// });
/// ```
#[wasm_bindgen]
pub struct Client {
    inner: nimiq::client::Client,

    /// The network ID that the client is connecting to.
    network_id: u8,

    /// A hashmap from address to the count of listeners subscribed to it
    subscribed_addresses: Rc<RefCell<HashMap<nimiq_keys::Address, u16>>>,

    listener_id: Cell<usize>,
    consensus_changed_listeners: Rc<RefCell<HashMap<usize, Function>>>,
    head_changed_listeners: Rc<RefCell<HashMap<usize, Function>>>,
    peer_changed_listeners: Rc<RefCell<HashMap<usize, Function>>>,
    transaction_listeners: Rc<RefCell<HashMap<usize, (Function, HashSet<nimiq_keys::Address>)>>>,

    /// Map from transaction hash as hex string to oneshot sender.
    /// Used to await transaction events in `send_transaction`.
    transaction_oneshots: Rc<RefCell<HashMap<String, oneshot::Sender<PlainTransactionDetails>>>>,
}

#[wasm_bindgen]
impl Client {
    /// Creates a new Client that automatically starts connecting to the network.
    pub async fn create(config: &PlainClientConfigurationType) -> Result<Client, JsError> {
        let plain_config: PlainClientConfiguration =
            serde_wasm_bindgen::from_value((*config).clone())?;
        let web_config = ClientConfiguration::try_from(plain_config)?;

        let log_settings = LogSettings {
            level: Some(LevelFilter::from_str(web_config.log_level.as_str())?),
            ..Default::default()
        };

        // Initialize logging with config values.
        initialize_web_logging(Some(&log_settings)).expect("Web logging initialization failed");

        // Initialize panic hook.
        initialize_panic_reporting();

        log::info!(?web_config, "Web config");

        // Create config builder.
        let mut builder = ClientConfig::builder();

        // Finalize config.
        let mut config = builder
            .volatile()
            .light()
            .build()
            .expect("Build configuration failed");

        // Set the seed nodes
        let seed_nodes = web_config
            .seed_nodes
            .iter()
            .map(|seed| {
                Ok(Seed {
                    address: Multiaddr::from_str(seed)?,
                })
            })
            .collect::<Result<Vec<Seed>, JsError>>()?;

        config.network.seeds = seed_nodes;
        config.network.only_secure_ws_connections = true;
        config.network_id = web_config.network_id;
        config.network.desired_peer_count = 6;

        log::info!(?config, "Final configuration");

        // Create client from config.
        log::info!("Initializing light client");
        let mut client = nimiq::client::Client::from_config(config)
            .await
            .expect("Client initialization failed");
        log::info!("Web client initialized");

        // Start consensus.
        let consensus = client.take_consensus().unwrap();
        log::info!("Spawning consensus");
        spawn_local(consensus);

        let zkp_component = client.take_zkp_component().unwrap();
        spawn_local(zkp_component);

        let client = Client {
            inner: client,
            network_id: from_network_id(web_config.network_id),
            subscribed_addresses: Rc::new(RefCell::new(HashMap::new())),
            listener_id: Cell::new(0),
            consensus_changed_listeners: Rc::new(RefCell::new(HashMap::with_capacity(1))),
            head_changed_listeners: Rc::new(RefCell::new(HashMap::with_capacity(1))),
            peer_changed_listeners: Rc::new(RefCell::new(HashMap::with_capacity(1))),
            transaction_listeners: Rc::new(RefCell::new(HashMap::new())),
            transaction_oneshots: Rc::new(RefCell::new(HashMap::new())),
        };

        client.setup_offline_online_event_handlers();
        client.setup_consensus_events();
        client.setup_blockchain_events();
        client.setup_network_events();
        client.setup_transaction_events().await;

        Ok(client)
    }

    /// Adds an event listener for consensus-change events, such as when consensus is established or lost.
    #[wasm_bindgen(js_name = addConsensusChangedListener)]
    pub async fn add_consensus_changed_listener(
        &self,
        listener: ConsensusChangedListener,
    ) -> Result<usize, JsError> {
        let listener = listener
            .dyn_into::<Function>()
            .map_err(|_| JsError::new("listener is not a function"))?;

        let listener_id = self.next_listener_id();
        self.consensus_changed_listeners
            .borrow_mut()
            .insert(listener_id, listener);
        Ok(listener_id)
    }

    /// Adds an event listener for new blocks added to the blockchain.
    #[wasm_bindgen(js_name = addHeadChangedListener)]
    pub async fn add_head_changed_listener(
        &self,
        listener: HeadChangedListener,
    ) -> Result<usize, JsError> {
        let listener = listener
            .dyn_into::<Function>()
            .map_err(|_| JsError::new("listener is not a function"))?;

        let listener_id = self.next_listener_id();
        self.head_changed_listeners
            .borrow_mut()
            .insert(listener_id, listener);
        Ok(listener_id)
    }

    /// Adds an event listener for peer-change events, such as when a new peer joins, or a peer leaves.
    #[wasm_bindgen(js_name = addPeerChangedListener)]
    pub async fn add_peer_changed_listener(
        &self,
        listener: PeerChangedListener,
    ) -> Result<usize, JsError> {
        let listener = listener
            .dyn_into::<Function>()
            .map_err(|_| JsError::new("listener is not a function"))?;

        let listener_id = self.next_listener_id();
        self.peer_changed_listeners
            .borrow_mut()
            .insert(listener_id, listener);
        Ok(listener_id)
    }

    /// Adds an event listener for transactions to and from the provided addresses.
    ///
    /// The listener is called for transactions when they are _included_ in the blockchain.
    #[wasm_bindgen(js_name = addTransactionListener)]
    pub async fn add_transaction_listener(
        &self,
        listener: TransactionListener,
        addresses: &AddressAnyArrayType,
    ) -> Result<usize, JsError> {
        let listener = listener
            .dyn_into::<Function>()
            .map_err(|_| JsError::new("listener is not a function"))?;

        let addresses: HashSet<_, _> = Client::unpack_addresses(addresses)?.into_iter().collect();

        // Add addresses to our global list of subscribed addresses
        {
            // Borrow RefCell in a new scope, as Clippy did not detect usage of drop(...).
            let mut subscribed_addresses = self.subscribed_addresses.borrow_mut();
            for address in addresses.iter() {
                subscribed_addresses
                    .entry(address.clone())
                    .and_modify(|count| *count += 1)
                    .or_insert(1);
            }
        }

        // Add to our listeners
        let listener_id = self.next_listener_id();
        self.transaction_listeners
            .borrow_mut()
            .insert(listener_id, (listener, addresses.clone()));

        // Then subscribe at network
        // Ignore failure because we still want to return the listener ID to the caller.
        let _ = self
            .inner
            .consensus_proxy()
            .subscribe_to_addresses(addresses.into_iter().collect(), 1, None)
            .await;

        Ok(listener_id)
    }

    /// Removes an event listener by its handle.
    #[wasm_bindgen(js_name = removeListener)]
    pub async fn remove_listener(&self, handle: usize) {
        self.consensus_changed_listeners
            .borrow_mut()
            .remove(&handle);
        self.head_changed_listeners.borrow_mut().remove(&handle);
        self.peer_changed_listeners.borrow_mut().remove(&handle);

        if let Some((_, unsubscribed_addresses)) =
            self.transaction_listeners.borrow_mut().remove(&handle)
        {
            let mut subscribed_addresses = self.subscribed_addresses.borrow_mut();
            let mut removed_addresses = vec![];
            for unsubscribed_address in unsubscribed_addresses {
                if let Entry::Occupied(mut entry) =
                    subscribed_addresses.entry(unsubscribed_address.clone())
                {
                    *entry.get_mut() -= 1;

                    if entry.get() == &0 {
                        entry.remove_entry();
                        removed_addresses.push(unsubscribed_address);
                    }
                }
            }
            if !removed_addresses.is_empty() {
                let owned_consensus = self.inner.consensus_proxy();
                spawn_local(async move {
                    let _ = owned_consensus
                        .unsubscribe_from_addresses(removed_addresses, 1)
                        .await;
                });
            }
        }
    }

    /// Returns the network ID that the client is connecting to.
    #[wasm_bindgen(js_name = getNetworkId)]
    pub async fn get_network_id(&self) -> u8 {
        self.network_id
    }

    /// Returns if the client currently has consensus with the network.
    #[wasm_bindgen(js_name = isConsensusEstablished)]
    pub async fn is_consensus_established(&self) -> bool {
        self.inner.consensus_proxy().is_established()
    }

    /// Returns a promise that resolves when the client has established consensus with the network.
    #[wasm_bindgen(js_name = waitForConsensusEstablished)]
    pub async fn wait_for_consensus_established(&self) -> Result<(), JsError> {
        if self.is_consensus_established().await {
            return Ok(());
        }

        let is_established = self
            .inner
            .consensus_proxy()
            .subscribe_events()
            .any(|event| async move {
                if let Ok(state) = event {
                    matches!(state, ConsensusEvent::Established { .. })
                } else {
                    self.is_consensus_established().await
                }
            })
            .await;

        if !is_established {
            // The stream terminated before an `Established` event occurred
            return Err(JsError::new("Stream ended"));
        }

        Ok(())
    }

    /// Returns the block hash of the current blockchain head.
    #[wasm_bindgen(js_name = getHeadHash)]
    pub async fn get_head_hash(&self) -> String {
        self.inner.blockchain_head().hash().to_hex()
    }

    /// Returns the block number of the current blockchain head.
    #[wasm_bindgen(js_name = getHeadHeight)]
    pub async fn get_head_height(&self) -> u32 {
        self.inner.blockchain_head().block_number()
    }

    /// Returns the current blockchain head block.
    /// Note that the web client is a light client and does not have block bodies, i.e. no transactions.
    #[wasm_bindgen(js_name = getHeadBlock)]
    pub async fn get_head_block(&self) -> Result<PlainBlockType, JsError> {
        let block = self.inner.blockchain_head();
        Ok(serde_wasm_bindgen::to_value(&PlainBlock::from_block(&block))?.into())
    }

    /// Returns the current address books peers.
    /// Each peer will have one address and currently no guarantee for the usefulness of that address can be given.
    ///
    /// The resulting Array may be empty if there is no peers in the address book.
    #[wasm_bindgen(js_name = getAddressBook)]
    pub async fn get_address_book(&self) -> PlainPeerInfoArrayType {
        let contacts: Vec<_> = self
            .inner
            .network()
            .get_address_book()
            .into_iter()
            .map(|(peer_id, peer_info)| PlainPeerInfo::from(peer_id.to_string(), peer_info))
            .collect();

        serde_wasm_bindgen::to_value(&contacts).unwrap().into()
    }

    /// Fetches a block by its hash.
    ///
    /// Throws if the client does not have the block.
    ///
    /// Fetching blocks from the network is not yet available.
    #[wasm_bindgen(js_name = getBlock)]
    pub async fn get_block(&self, hash: &str) -> Result<PlainBlockType, JsError> {
        let hash = Blake2bHash::from_str(hash)?;
        let block = self
            .inner
            .consensus_proxy()
            .blockchain
            .read()
            .get_block(&hash, false)?;
        Ok(serde_wasm_bindgen::to_value(&PlainBlock::from_block(&block))?.into())
    }

    /// Fetches a block by its height (block number).
    ///
    /// Throws if the client does not have the block.
    ///
    /// Fetching blocks from the network is not yet available.
    #[wasm_bindgen(js_name = getBlockAt)]
    pub async fn get_block_at(&self, height: u32) -> Result<PlainBlockType, JsError> {
        let block = self
            .inner
            .consensus_proxy()
            .blockchain
            .read()
            .get_block_at(height, false)?;
        Ok(serde_wasm_bindgen::to_value(&PlainBlock::from_block(&block))?.into())
    }

    /// Fetches the account for the provided address from the network.
    ///
    /// Throws if the address cannot be parsed and on network errors.
    #[wasm_bindgen(js_name = getAccount)]
    pub async fn get_account(&self, address: &AddressAnyType) -> Result<PlainAccountType, JsError> {
        let address = Address::from_any(address)?.take_native();
        let plain_accounts = self.get_plain_accounts(vec![address]).await?;
        let account = plain_accounts.first().unwrap();
        Ok(serde_wasm_bindgen::to_value(account)?.into())
    }

    /// Fetches the accounts for the provided addresses from the network.
    ///
    /// Throws if an address cannot be parsed and on network errors.
    #[wasm_bindgen(js_name = getAccounts)]
    pub async fn get_accounts(
        &self,
        addresses: &AddressAnyArrayType,
    ) -> Result<PlainAccountArrayType, JsError> {
        let addresses = Client::unpack_addresses(addresses)?;
        let plain_accounts = self.get_plain_accounts(addresses).await?;
        Ok(serde_wasm_bindgen::to_value(&plain_accounts)?.into())
    }

    /// Fetches the staker for the provided address from the network.
    ///
    /// Throws if the address cannot be parsed and on network errors.
    #[wasm_bindgen(js_name = getStaker)]
    pub async fn get_staker(&self, address: &AddressAnyType) -> Result<PlainStakerType, JsError> {
        let address = Address::from_any(address)?.take_native();
        let plain_stakers = self.get_plain_stakers(vec![address]).await?;
        let staker = plain_stakers.first().unwrap();
        Ok(serde_wasm_bindgen::to_value(staker)?.into())
    }

    /// Fetches the stakers for the provided addresses from the network.
    ///
    /// Throws if an address cannot be parsed and on network errors.
    #[wasm_bindgen(js_name = getStakers)]
    pub async fn get_stakers(
        &self,
        addresses: &AddressAnyArrayType,
    ) -> Result<PlainStakerArrayType, JsError> {
        let addresses = Client::unpack_addresses(addresses)?;
        let plain_stakers = self.get_plain_stakers(addresses).await?;
        Ok(serde_wasm_bindgen::to_value(&plain_stakers)?.into())
    }

    /// Fetches the validator for the provided address from the network.
    ///
    /// Throws if the address cannot be parsed and on network errors.
    #[wasm_bindgen(js_name = getValidator)]
    pub async fn get_validator(
        &self,
        address: &AddressAnyType,
    ) -> Result<PlainValidatorType, JsError> {
        let address = Address::from_any(address)?.take_native();
        let plain_validators = self.get_plain_validators(vec![address]).await?;
        let validator = plain_validators.first().unwrap();
        Ok(serde_wasm_bindgen::to_value(validator)?.into())
    }

    /// Fetches the validators for the provided addresses from the network.
    ///
    /// Throws if an address cannot be parsed and on network errors.
    #[wasm_bindgen(js_name = getValidators)]
    pub async fn get_validators(
        &self,
        addresses: &AddressAnyArrayType,
    ) -> Result<PlainValidatorArrayType, JsError> {
        let addresses = Client::unpack_addresses(addresses)?;
        let plain_validators = self.get_plain_validators(addresses).await?;
        Ok(serde_wasm_bindgen::to_value(&plain_validators)?.into())
    }

    /// Sends a transaction to the network and returns {@link PlainTransactionDetails}.
    ///
    /// Throws in case of network errors.
    #[wasm_bindgen(js_name = sendTransaction)]
    pub async fn send_transaction(
        &self,
        transaction: &TransactionAnyType,
    ) -> Result<PlainTransactionDetailsType, JsError> {
        let tx = Transaction::from_any(transaction)?;
        let details = self.send_web_transaction(tx).await?;
        Ok(serde_wasm_bindgen::to_value(&details)?.into())
    }

    async fn send_web_transaction(
        &self,
        tx: Transaction,
    ) -> Result<PlainTransactionDetails, JsError> {
        tx.verify(Some(self.network_id))?;

        // Check if we are already subscribed to the sender or recipient
        let already_subscribed = self
            .subscribed_addresses
            .borrow()
            // Check sender first, as apps are usually subscribed to the sender already
            .contains_key(tx.sender().native_ref())
            || self
                .subscribed_addresses
                .borrow()
                .contains_key(tx.recipient().native_ref());

        let consensus = self.inner.consensus_proxy();

        struct AddressSubscription {
            // Use Options to be able to take the address and consensus out of the
            // struct during drop, avoiding another clone.
            address: Option<nimiq_keys::Address>,
            consensus: Option<ConsensusProxy>,
        }
        impl AddressSubscription {
            pub async fn subscribe(
                address: nimiq_keys::Address,
                consensus: ConsensusProxy,
            ) -> Result<Self, JsError> {
                consensus
                    .subscribe_to_addresses(vec![address.clone()], 1, None)
                    .await?;
                Ok(Self {
                    address: Some(address),
                    consensus: Some(consensus),
                })
            }
        }
        impl Drop for AddressSubscription {
            fn drop(&mut self) {
                let address = self.address.take().unwrap();
                let consensus = self.consensus.take().unwrap();
                // Unsubscribe from the address without caring about the result
                spawn_local(async move {
                    let _ = consensus.unsubscribe_from_addresses(vec![address], 1).await;
                });
            }
        }
        let mut address_subscription: Option<AddressSubscription> = None;

        // If not subscribed, subscribe to the sender or recipient
        if !already_subscribed {
            // Subscribe to the recipient by default
            let mut subscribed_address = tx.recipient().native();
            if subscribed_address == Policy::STAKING_CONTRACT_ADDRESS {
                // If the recipient is the staking contract, subscribe to the sender instead
                // to not get flooded with notifications.
                subscribed_address = tx.sender().native();
            }
            address_subscription =
                Some(AddressSubscription::subscribe(subscribed_address, consensus.clone()).await?);
        }

        let hash = &tx.hash();

        // Set a oneshot sender to receive the transaction when its notification arrives
        let (sender, receiver) = oneshot::channel();
        self.transaction_oneshots
            .borrow_mut()
            .insert(hash.clone(), sender);

        // Actually send the transaction
        consensus.send_transaction(tx.native()).await?;

        let timeout = gloo_timers::future::TimeoutFuture::new(10_000);

        // Wait for the transaction (will be None if the timeout is reached first)
        let res = select(receiver, timeout).await;

        let maybe_details = if let Either::Left((res, _)) = res {
            res.ok()
        } else {
            // If the timeout triggered, delete our oneshot sender
            self.transaction_oneshots.borrow_mut().remove(hash);
            None
        };

        drop(address_subscription);

        if let Some(details) = maybe_details {
            // If we got a transactions, return it
            Ok(details)
        } else {
            // If the transaction did not get included, return it as `TransactionState::New`
            let details =
                PlainTransactionDetails::new(&tx, TransactionState::New, None, None, None, None);
            Ok(details)
        }
    }

    /// Fetches the transaction details for the given transaction hash.
    #[wasm_bindgen(js_name = getTransaction)]
    pub async fn get_transaction(
        &self,
        hash: String,
    ) -> Result<PlainTransactionDetailsType, JsError> {
        let hash =
            Blake2bHash::from_str(&hash).map_err(|_| JsError::new("Invalid transaction hash"))?;
        let details = self
            .inner
            .consensus_proxy()
            .prove_transactions_from_receipts(vec![(hash, None)], 1)
            .await?
            .into_iter()
            .next()
            .map(|hist_tx| {
                PlainTransactionDetails::try_from_historic_transaction(
                    hist_tx,
                    self.inner.blockchain_head().block_number(),
                )
                .expect("no non-reward inherent")
            })
            .ok_or_else(|| JsError::new("Transaction not found"))?;
        Ok(serde_wasm_bindgen::to_value(&details)?.into())
    }

    /// This function is used to query the network for transaction receipts from and to a
    /// specific address, that have been included in the chain.
    ///
    /// The obtained receipts are _not_ verified before being returned.
    ///
    /// Up to a `limit` number of transaction receipts are returned from newest to oldest.
    /// It starts at the `start_at` transaction and goes backwards. If this hash does not exist
    /// or does not belong to the address, an empty list is returned.
    /// If the network does not have at least `min_peers` to query, then an error is returned.
    #[wasm_bindgen(js_name = getTransactionReceiptsByAddress)]
    pub async fn get_transaction_receipts_by_address(
        &self,
        address: &AddressAnyType,
        limit: Option<u16>,
        start_at: Option<String>,
        min_peers: Option<usize>,
    ) -> Result<PlainTransactionReceiptArrayType, JsError> {
        if let Some(max) = limit {
            if max > MAX_TRANSACTIONS_BY_ADDRESS {
                return Err(JsError::new(
                    "The maximum number of transaction receipts exceeds the one that is supported",
                ));
            }
        }

        let start_at = if let Some(start_at) = start_at {
            Some(Blake2bHash::from_str(&start_at)?)
        } else {
            None
        };

        let receipts = self
            .inner
            .consensus_proxy()
            .request_transaction_receipts_by_address(
                Address::from_any(address)?.take_native(),
                min_peers.unwrap_or(1),
                limit,
                start_at,
            )
            .await?;

        let plain_tx_receipts: Vec<_> = receipts
            .into_iter()
            .map(|receipt| PlainTransactionReceipt::from_receipt(&receipt))
            .collect();

        Ok(serde_wasm_bindgen::to_value(&plain_tx_receipts)?.into())
    }

    /// This function is used to query the network for transactions from and to a specific
    /// address, that have been included in the chain.
    ///
    /// The obtained transactions are verified before being returned.
    ///
    /// If you already have transactions belonging to this address, you can provide some of that
    /// information to reduce the amount of network requests made:
    /// - Provide the `since_block_height` parameter to exclude any history from before
    ///   that block height. You should be completely certain about its state. This should not be
    ///   the last known block height, but an earlier block height that could not have been forked
    ///   from (e.g. the last known election or checkpoint block).
    /// - Provide a list of `known_transaction_details` to have them verified and/or broadcasted
    ///   again.
    /// - Provide a `start_at` parameter to start the query at a specific transaction hash
    ///   (which will not be included). This hash must exist and the corresponding transaction
    ///   must involve this address for the query to work correctly.
    ///
    /// Up to a `limit` number of transactions are returned from newest to oldest.
    /// If the network does not have at least `min_peers` to query, an error is returned.
    #[wasm_bindgen(js_name = getTransactionsByAddress)]
    pub async fn get_transactions_by_address(
        &self,
        address: &AddressAnyType,
        since_block_height: Option<u32>,
        known_transaction_details: Option<PlainTransactionDetailsArrayType>,
        start_at: Option<String>,
        limit: Option<u16>,
        min_peers: Option<usize>,
    ) -> Result<PlainTransactionDetailsArrayType, JsError> {
        let since_block_height = since_block_height.unwrap_or(0);
        let min_peers = min_peers.unwrap_or(1);

        let start_at = if let Some(start_at) = start_at {
            Some(Blake2bHash::from_str(&start_at)?)
        } else {
            None
        };

        if let Some(max) = limit {
            if max > MAX_TRANSACTIONS_BY_ADDRESS {
                return Err(JsError::new(
                    "The maximum number of transactions exceeds the one that is supported",
                ));
            }
        }

        let address = Address::from_any(address)?.take_native();
        let mut known_txs = HashMap::new();

        // Parse known transaction details from JS to Rust
        if let Some(array) = known_transaction_details {
            let known_transaction_details =
                serde_wasm_bindgen::from_value::<Vec<PlainTransactionDetails>>(array.into())?;
            for details in known_transaction_details {
                known_txs.insert(
                    Blake2bHash::from_str(&details.transaction.transaction_hash)?,
                    details,
                );
            }
        }

        // Fetch transaction receipts.
        let receipts: HashMap<_, _> = self
            .inner
            .consensus_proxy()
            .request_transaction_receipts_by_address(address, min_peers, limit, start_at)
            .await?
            .into_iter()
            .collect();

        let mut receipts_to_fetch = vec![];
        for (hash, block_number) in receipts.iter() {
            // Skip known transactions that are already considered confirmed.
            if let Some(known_tx) = known_txs.get(hash) {
                if matches!(known_tx.state, TransactionState::Confirmed)
                    && known_tx.block_height == Some(*block_number)
                {
                    continue;
                }
            }

            // Ignore all receipts that are older than since_block_height.
            if block_number < &since_block_height {
                continue;
            }

            // Add the transaction to the list of transactions to fetch.
            receipts_to_fetch.push((hash.clone(), Some(*block_number)));
        }

        // Re-check known included or confirmed transactions that are not contained in the receipts.
        for (hash, details) in &known_txs {
            if !matches!(
                details.state,
                TransactionState::Included | TransactionState::Confirmed
            ) {
                continue;
            }
            if receipts.contains_key(hash) {
                // Transaction is included in receipts, so not unknown.
                continue;
            }
            // If the known transaction was earlier than the configured cutoff or the earliest
            // retrievable receipt, do not try to verify it.
            if let Some(block_height) = details.block_height {
                if block_height < since_block_height {
                    continue;
                }
            }

            receipts_to_fetch.push((hash.clone(), details.block_height));
        }

        // Retrieve proofs for the transactions we want to check.
        let ext_txs = self
            .inner
            .consensus_proxy()
            .prove_transactions_from_receipts(receipts_to_fetch, min_peers)
            .await?;

        let current_height = self.get_head_height().await;

        // Convert historic transactions into the plain transaction details result type.
        let mut txs: Vec<_> = ext_txs
            .into_iter()
            .map(|hist_tx| {
                // This method automatically marks the PlainTransactionDetails as status `Included`
                // or `confirmed` from the `current_height`.
                PlainTransactionDetails::try_from_historic_transaction(hist_tx, current_height)
                    .expect("no non-reward inherent")
            })
            .collect();

        // Track known new or pending transactions.
        for details in known_txs.values() {
            if !matches!(
                details.state,
                TransactionState::New | TransactionState::Pending
            ) {
                continue;
            }
            if txs
                .iter()
                .any(|tx| tx.transaction.transaction_hash == details.transaction.transaction_hash)
            {
                // We already fetched this transaction, meaning it got included and will be returned.
                continue;
            }

            if details.transaction.validity_start_height
                <= current_height - Policy::transaction_validity_window()
            {
                // Transaction expired before getting included.
                let mut expired_details = details.clone();
                expired_details.state = TransactionState::Expired;
                txs.push(expired_details);
            } else {
                // Re-broadcast the transaction and return it with its new state.
                txs.push(
                    self.send_web_transaction(Transaction::from_plain_transaction(
                        &details.transaction,
                    )?)
                    .await?,
                );
            }
        }

        Ok(serde_wasm_bindgen::to_value(&txs)?.into())
    }

    /// This function is used to tell the network to disconnect from every connected
    /// peer and stop trying to connect to other peers.
    ///
    /// **Important**: this function returns when the signal to disconnect was sent,
    /// before all peers actually disconnect. This means that in order to ensure the
    /// network is disconnected, wait for all peers to disappear after calling.
    #[wasm_bindgen(js_name = disconnectNetwork)]
    pub async fn disconnect_network(&self) {
        let network = self.inner.network();
        network.disconnect(CloseReason::GoingOffline).await;
    }

    /// This function is used to tell the network to (re)start connecting to peers.
    /// This is could be used to tell the network to restart connection operations after
    /// disconnect network is called.
    #[wasm_bindgen(js_name = connectNetwork)]
    pub async fn connect_network(&self) {
        let network = self.inner.network();
        network.start_connecting().await;
    }

    fn setup_offline_online_event_handlers(&self) {
        let network = self.inner.network();

        // Register online/offline/visible closure
        let handler = Closure::<dyn Fn(MessageEvent)>::new(move |event: MessageEvent| {
            if let Some(state) = event.data().dyn_ref::<JsString>() {
                if state == &JsString::from_str("offline").unwrap() {
                    log::warn!("Network went offline");
                    let network = network.clone();
                    spawn_local(async move {
                        let network = network.clone();
                        network.disconnect(CloseReason::GoingOffline).await;
                    });
                } else if state == &JsString::from_str("online").unwrap() {
                    log::warn!("Network went online");
                    let network = network.clone();
                    spawn_local(async move {
                        let network = network.clone();
                        network.start_connecting().await;
                    });
                } else if state == &JsString::from_str("visible").unwrap() {
                    log::debug!("Content became visible: restarting network");
                    let network = network.clone();
                    spawn_local(async move {
                        let network = network.clone();
                        network.start_connecting().await;
                    });
                }
            }
        });

        let _ = add_event_listener("message", handler.as_ref().unchecked_ref()).map_err(|err| {
            // TODO: When on NodeJS, call `addListener` instead
            log::warn!(
                "Unable to set event listener for 'message' event: {:?}",
                err
            );
        });

        // Closures can't be dropped since they will be needed outside the context
        // of this function
        handler.forget();
    }

    fn setup_consensus_events(&self) {
        let consensus = self.inner.consensus_proxy();
        let network = self.inner.network();

        let mut consensus_events = consensus.subscribe_events();

        let consensus_listeners = Rc::clone(&self.consensus_changed_listeners);

        spawn_local(async move {
            loop {
                let state = match consensus_events.next().await {
                    Some(Ok(ConsensusEvent::Established { .. })) => {
                        Some(ConsensusState::Established)
                    }
                    Some(Ok(ConsensusEvent::Lost)) => {
                        if network.peer_count() >= 1 {
                            Some(ConsensusState::Syncing)
                        } else {
                            Some(ConsensusState::Connecting)
                        }
                    }
                    Some(Err(_)) => {
                        None // Ignore stream errors
                    }
                    None => {
                        break;
                    }
                };

                if let Some(state) = state {
                    Client::fire_consensus_event(&consensus_listeners, state);
                }
            }
        });
    }

    fn fire_consensus_event(
        listeners: &Rc<RefCell<HashMap<usize, Function>>>,
        state: ConsensusState,
    ) {
        let state = JsValue::from(state.to_string());

        let this = JsValue::null();
        for listener in listeners.borrow().values() {
            let _ = listener.call1(&this, &state);
        }
    }

    fn setup_blockchain_events(&self) {
        let blockchain = self.inner.consensus_proxy().blockchain;

        let mut blockchain_events = blockchain.read().notifier_as_stream();

        let block_listeners = Rc::clone(&self.head_changed_listeners);

        spawn_local(async move {
            loop {
                let (hash, reason, reverted_blocks, adopted_blocks) =
                    match blockchain_events.next().await {
                        Some(BlockchainEvent::Extended(hash)) => {
                            let adopted_blocks = Array::new();
                            adopted_blocks.push(&hash.to_hex().into());

                            (hash, "extended", Array::new(), adopted_blocks)
                        }
                        Some(BlockchainEvent::HistoryAdopted(hash)) => {
                            let adopted_blocks = Array::new();
                            adopted_blocks.push(&hash.to_hex().into());

                            (hash, "history-adopted", Array::new(), adopted_blocks)
                        }
                        Some(BlockchainEvent::EpochFinalized(hash)) => {
                            let adopted_blocks = Array::new();
                            adopted_blocks.push(&hash.to_hex().into());

                            (hash, "epoch-finalized", Array::new(), adopted_blocks)
                        }
                        Some(BlockchainEvent::Finalized(hash)) => {
                            let adopted_blocks = Array::new();
                            adopted_blocks.push(&hash.to_hex().into());

                            (hash, "finalized", Array::new(), adopted_blocks)
                        }
                        Some(BlockchainEvent::Rebranched(old_chain, new_chain)) => {
                            let hash = &new_chain.last().unwrap().0.clone();

                            let reverted_blocks = Array::new();
                            for (h, _) in old_chain {
                                reverted_blocks.push(&h.to_hex().into());
                            }

                            let adopted_blocks = Array::new();
                            for (h, _) in new_chain {
                                adopted_blocks.push(&h.to_hex().into());
                            }

                            (
                                hash.to_owned(),
                                "rebranched",
                                reverted_blocks,
                                adopted_blocks,
                            )
                        }
                        Some(BlockchainEvent::Stored(block)) => {
                            (block.hash(), "stored", Array::new(), Array::new())
                        }
                        None => {
                            break;
                        }
                    };

                let args = Array::new();
                args.push(&hash.to_hex().into());
                args.push(&reason.into());
                args.push(&reverted_blocks);
                args.push(&adopted_blocks);

                let this = JsValue::null();
                for listener in block_listeners.borrow().values() {
                    let _ = listener.apply(&this, &args);
                }
            }
        });
    }

    fn setup_network_events(&self) {
        let network = self.inner.network();
        let consensus = self.inner.consensus_proxy();

        let mut network_events = network.subscribe_events();

        let subscribed_addresses = Rc::clone(&self.subscribed_addresses);

        let peer_listeners = Rc::clone(&self.peer_changed_listeners);
        let consensus_listeners = Rc::clone(&self.consensus_changed_listeners);

        spawn_local(async move {
            loop {
                let details = match network_events.next().await {
                    Some(Ok(NetworkEvent::PeerJoined(peer_id, peer_info))) => {
                        if subscribed_addresses.borrow().len() > 0 {
                            // Subscribe to all addresses at the new peer
                            let owned_consensus = consensus.clone();
                            let owned_subscribed_addresses = Rc::clone(&subscribed_addresses);
                            let addresses = owned_subscribed_addresses
                                .borrow()
                                .keys()
                                .cloned()
                                .collect();
                            spawn_local(async move {
                                let _ = owned_consensus
                                    .subscribe_to_addresses(addresses, 1, Some(peer_id))
                                    .await;
                                log::debug!(
                                    peer_id=%peer_id,
                                    "Subscribed to {} addresses at new peer",
                                    owned_subscribed_addresses.borrow().len(),
                                );
                            });
                        }

                        Some((
                            peer_id.to_string(),
                            "joined",
                            Some(
                                serde_wasm_bindgen::to_value(&PlainPeerInfo::from(
                                    peer_id.to_string(),
                                    peer_info,
                                ))
                                .unwrap(),
                            ),
                        ))
                    }
                    Some(Ok(NetworkEvent::PeerLeft(peer_id))) => {
                        Some((peer_id.to_string(), "left", None))
                    }
                    Some(_) => {
                        None // Ignore stream errors and other events
                    }
                    None => {
                        break;
                    }
                };

                let peer_count = network.peer_count();

                if !consensus.is_established() {
                    if peer_count >= 1 {
                        Client::fire_consensus_event(&consensus_listeners, ConsensusState::Syncing)
                    } else {
                        Client::fire_consensus_event(
                            &consensus_listeners,
                            ConsensusState::Connecting,
                        )
                    }
                }

                if let Some((peer_id, reason, peer_info)) = details {
                    let args = Array::new();
                    args.push(&peer_id.into());
                    args.push(&reason.into());
                    args.push(&peer_count.into());
                    args.push(&peer_info.into());

                    let this = JsValue::null();
                    for listener in peer_listeners.borrow().values() {
                        let _ = listener.apply(&this, &args);
                    }
                }
            }
        });
    }

    async fn setup_transaction_events(&self) {
        let consensus = self.inner.consensus_proxy();

        let transaction_listeners = Rc::clone(&self.transaction_listeners);
        let transaction_oneshots = Rc::clone(&self.transaction_oneshots);

        spawn_local(async move {
            let mut address_notifications = consensus.subscribe_address_notifications().await;

            while let Some((notification, _)) = address_notifications.next().await {
                {
                    loop {
                        let current_block_number =
                            consensus.blockchain.read().head().block_number();
                        if notification
                            .receipts
                            .iter()
                            .any(|(_, block_number)| block_number > &current_block_number)
                        {
                            log::debug!("Received transaction receipt(s) from the future, waiting for the blockchain head to be updated...");
                            let mut blockchain_events =
                                consensus.blockchain.read().notifier_as_stream();
                            let _ = blockchain_events.next().await;
                        } else {
                            break;
                        }
                    }
                }

                let receipts = notification
                    .receipts
                    .into_iter()
                    .map(|(hash, block_number)| (hash, Some(block_number)))
                    .collect();

                if let Ok(hist_txs) = consensus
                    .prove_transactions_from_receipts(receipts, 1)
                    .await
                    .map_err(|e| {
                        log::error!("Failed to prove transactions from receipts: {}", e);
                    })
                {
                    let this = JsValue::null();

                    for hist_tx in hist_txs {
                        let block_number = hist_tx.block_number;
                        let details = PlainTransactionDetails::try_from_historic_transaction(
                            hist_tx,
                            block_number,
                        )
                        .expect("no non-reward inherent");

                        if let Some(sender) = transaction_oneshots
                            .borrow_mut()
                            .remove(&details.transaction.transaction_hash)
                        {
                            let _ = sender.send(details.clone());
                        }

                        fn from_user(addr: &str) -> nimiq_keys::Address {
                            nimiq_keys::Address::from_user_friendly_address(addr).unwrap()
                        }

                        let sender = from_user(&details.transaction.sender);
                        let recipient = from_user(&details.transaction.recipient);
                        let staker_address = if let PlainTransactionRecipientData::AddStake(data) =
                            &details.transaction.data
                        {
                            Some(from_user(&data.staker))
                        } else {
                            None
                        };

                        if let Ok(js_value) = serde_wasm_bindgen::to_value(&details) {
                            for (listener, addresses) in transaction_listeners.borrow().values() {
                                if addresses.contains(&sender)
                                    || addresses.contains(&recipient)
                                    || if let Some(ref address) = staker_address {
                                        addresses.contains(address)
                                    } else {
                                        false
                                    }
                                {
                                    let _ = listener.call1(&this, &js_value);
                                }
                            }
                        }
                    }
                }
            }
        });
    }

    fn next_listener_id(&self) -> usize {
        let mut id = self.listener_id.get();
        id += 1;
        self.listener_id.set(id);
        id
    }
}

impl Client {
    fn unpack_addresses(
        addresses: &AddressAnyArrayType,
    ) -> Result<Vec<nimiq_keys::Address>, JsError> {
        // Unpack the array of addresses
        let js_value: &JsValue = addresses.unchecked_ref();
        let array: &Array = js_value
            .dyn_ref()
            .ok_or_else(|| JsError::new("`addresses` must be an array"))?;

        if array.length() == 0 {
            return Err(JsError::new("No addresses provided"));
        }

        let mut addresses = Vec::<_>::with_capacity(array.length().try_into()?);
        for any in array.iter() {
            let address = Address::from_any(&any.into())?.take_native();
            addresses.push(address);
        }

        Ok(addresses)
    }

    async fn get_plain_accounts(
        &self,
        addresses: Vec<nimiq_keys::Address>,
    ) -> Result<Vec<PlainAccount>, JsError> {
        let accounts = self
            .inner
            .consensus_proxy()
            .request_accounts_by_addresses(addresses.clone(), 1)
            .await?;

        let mut ordered_accounts = vec![];
        let default = nimiq_account::Account::default();

        for address in &addresses {
            ordered_accounts.push(PlainAccount::from(
                accounts
                    .get(address)
                    .ok_or(JsError::new(&format!(
                        "Missing trie proof node for {}",
                        address
                    )))?
                    .as_ref()
                    .unwrap_or(&default),
            ));
        }

        Ok(ordered_accounts)
    }

    async fn get_plain_stakers(
        &self,
        addresses: Vec<nimiq_keys::Address>,
    ) -> Result<Vec<Option<PlainStaker>>, JsError> {
        let stakers = self
            .inner
            .consensus_proxy()
            .request_stakers_by_addresses(addresses.clone(), 1)
            .await?;

        let mut ordered_stakers = vec![];

        for address in &addresses {
            ordered_stakers.push(
                stakers
                    .get(address)
                    .ok_or(JsError::new(&format!(
                        "Missing trie proof node for {}",
                        address
                    )))?
                    .as_ref()
                    .map(PlainStaker::from),
            );
        }

        Ok(ordered_stakers)
    }

    async fn get_plain_validators(
        &self,
        addresses: Vec<nimiq_keys::Address>,
    ) -> Result<Vec<Option<PlainValidator>>, JsError> {
        let validators = self
            .inner
            .consensus_proxy()
            .request_validators_by_addresses(addresses.clone(), 1)
            .await?;

        let mut ordered_validators = vec![];

        for address in &addresses {
            ordered_validators.push(
                validators
                    .get(address)
                    .ok_or(JsError::new(&format!(
                        "Missing trie proof node for {}",
                        address
                    )))?
                    .as_ref()
                    .map(PlainValidator::from),
            );
        }

        Ok(ordered_validators)
    }
}

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(typescript_type = "(state: ConsensusState) => any")]
    pub type ConsensusChangedListener;

    #[wasm_bindgen(
        typescript_type = "(hash: string, reason: string, reverted_blocks: string[], adopted_blocks: string[]) => any"
    )]
    pub type HeadChangedListener;

    #[wasm_bindgen(
        typescript_type = "(peer_id: string, reason: 'joined' | 'left', peer_count: number, peer_info?: PlainPeerInfo) => any"
    )]
    pub type PeerChangedListener;

    #[wasm_bindgen(typescript_type = "(transaction: PlainTransactionDetails) => any")]
    pub type TransactionListener;
}

#[wasm_bindgen]
extern "C" {
    pub type GlobalScope;

    #[wasm_bindgen(catch, method, js_name = addEventListener)]
    pub fn add_event_listener_with_callback(
        this: &GlobalScope,
        event: &str,
        callback: &Function,
    ) -> Result<(), JsValue>;
}

fn add_event_listener(event: &str, callback: &Function) -> Result<(), JsValue> {
    let global_this = global();
    let global_scope = global_this.unchecked_ref::<GlobalScope>();
    global_scope.add_event_listener_with_callback(event, callback)
}
