extern crate alloc; // Required for wasm-bindgen-derive

use std::{cell::RefCell, collections::HashMap, rc::Rc, str::FromStr};

use futures::StreamExt;
use js_sys::{Array, Date, Promise};
use log::level_filters::LevelFilter;
use tsify::Tsify;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::{spawn_local, JsFuture};

pub use nimiq::{
    client::Consensus,
    config::command_line::CommandLine,
    config::config::ClientConfig,
    config::config_file::{ConfigFile, LogSettings, Seed, SyncMode},
    error::Error,
    extras::{panic::initialize_panic_reporting, web_logging::initialize_web_logging},
};

use nimiq_blockchain_interface::{AbstractBlockchain, BlockchainEvent};
use nimiq_consensus::ConsensusEvent;
use nimiq_hash::Blake2bHash;
use nimiq_network_interface::{
    network::{CloseReason, Network, NetworkEvent},
    Multiaddr,
};
use nimiq_primitives::networks::NetworkId;

use crate::account::{PlainAccount, PlainAccountArrayType, PlainAccountType};
use crate::address::{Address, AddressAnyArrayType, AddressAnyType};
use crate::block::{PlainBlock, PlainBlockType};
use crate::peer_info::PeerInfo;
use crate::transaction::{
    PlainTransactionDetails, PlainTransactionDetailsArrayType, PlainTransactionDetailsType,
    PlainTransactionReceipt, PlainTransactionReceiptArrayType, Transaction, TransactionAnyType,
    TransactionState,
};
use crate::transaction_builder::TransactionBuilder;
use crate::utils::from_network_id;

mod account;
mod address;
mod block;
mod key_pair;
mod peer_info;
mod private_key;
mod public_key;
mod signature;
mod signature_proof;
mod transaction;
mod transaction_builder;
mod utils;

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

/// Use this to provide initialization-time configuration to the Client.
/// This is a simplified version of the configuration that is used for regular nodes,
/// since not all configuration knobs are available when running inside a browser.
#[wasm_bindgen]
pub struct ClientConfiguration {
    network_id: NetworkId,
    seed_nodes: Vec<String>,
    log_level: String,
}

impl Default for ClientConfiguration {
    fn default() -> Self {
        Self {
            network_id: NetworkId::DevAlbatross,
            seed_nodes: vec![],
            log_level: "info".to_string(),
        }
    }
}

#[wasm_bindgen]
impl ClientConfiguration {
    /// Creates a default client configuration that can be used to change the client's configuration.
    ///
    /// Use its `instantiateClient()` method to launch the client and connect to the network.
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        ClientConfiguration::default()
    }

    /// Sets the network ID the client should use. Input is case-insensitive.
    ///
    /// Possible values are `'TestAlbatross' | 'DevAlbatross'`.
    /// Default is `'DevAlbatross'`.
    pub fn network(&mut self, network: String) -> Result<(), JsError> {
        self.network_id = NetworkId::from_str(&network)?;
        Ok(())
    }

    /// Sets the list of seed nodes that are used to connect to the Nimiq Albatross network.
    ///
    /// Each array entry must be a proper Multiaddr format string.
    #[wasm_bindgen(js_name = seedNodes)]
    #[allow(clippy::boxed_local)]
    pub fn seed_nodes(&mut self, seeds: Box<[JsValue]>) {
        self.seed_nodes = seeds
            .iter()
            .map(|seed| serde_wasm_bindgen::from_value(seed.clone()).unwrap())
            .collect::<Vec<String>>();
    }

    /// Sets the log level that is used when logging to the console.
    ///
    /// Possible values are `'trace' | 'debug' | 'info' | 'warn' | 'error'`.
    /// Default is `'info'`.
    #[wasm_bindgen(js_name = logLevel)]
    pub fn log_level(&mut self, log_level: String) {
        self.log_level = log_level.to_lowercase();
    }

    /// Instantiates a client from this configuration builder.
    #[wasm_bindgen(js_name = instantiateClient)]
    pub async fn instantiate_client(&self) -> Client {
        Client::create(self).await
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
    #[wasm_bindgen(readonly, js_name = networkId)]
    pub network_id: u8,

    listener_id: usize,
    consensus_changed_listeners: Rc<RefCell<HashMap<usize, js_sys::Function>>>,
    head_changed_listeners: Rc<RefCell<HashMap<usize, js_sys::Function>>>,
    peer_changed_listeners: Rc<RefCell<HashMap<usize, js_sys::Function>>>,
}

#[wasm_bindgen]
impl Client {
    /// Creates a new Client that automatically starts connecting to the network.
    pub async fn create(web_config: &ClientConfiguration) -> Self {
        let log_settings = LogSettings {
            level: Some(LevelFilter::from_str(web_config.log_level.as_str()).unwrap()),
            ..Default::default()
        };

        // Initialize logging with config values.
        initialize_web_logging(Some(&log_settings)).expect("Web logging initialization failed");

        // Initialize panic hook.
        initialize_panic_reporting();

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
            .map(|seed| Seed {
                address: Multiaddr::from_str(seed).unwrap(),
            })
            .collect::<Vec<Seed>>();

        config.network.seeds = seed_nodes;
        config.network_id = web_config.network_id;

        log::debug!(?config, "Final configuration");

        // Create client from config.
        log::info!("Initializing light client");
        let mut client = nimiq::client::Client::from_config(
            config,
            Box::new(|fut| {
                spawn_local(fut);
            }),
        )
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
            listener_id: 0,
            consensus_changed_listeners: Rc::new(RefCell::new(HashMap::with_capacity(1))),
            head_changed_listeners: Rc::new(RefCell::new(HashMap::with_capacity(1))),
            peer_changed_listeners: Rc::new(RefCell::new(HashMap::with_capacity(1))),
        };

        client.setup_offline_online_event_handlers();
        client.setup_consensus_events();
        client.setup_blockchain_events();
        client.setup_network_events();

        client
    }

    /// Adds an event listener for consensus-change events, such as when consensus is established or lost.
    #[wasm_bindgen(js_name = addConsensusChangedListener)]
    pub async fn add_consensus_changed_listener(
        &mut self,
        listener: ConsensusChangedListener,
    ) -> Result<usize, JsError> {
        let listener = listener
            .dyn_into::<js_sys::Function>()
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
        &mut self,
        listener: HeadChangedListener,
    ) -> Result<usize, JsError> {
        let listener = listener
            .dyn_into::<js_sys::Function>()
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
        &mut self,
        listener: PeerChangedListener,
    ) -> Result<usize, JsError> {
        let listener = listener
            .dyn_into::<js_sys::Function>()
            .map_err(|_| JsError::new("listener is not a function"))?;

        let listener_id = self.next_listener_id();
        self.peer_changed_listeners
            .borrow_mut()
            .insert(listener_id, listener);
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
    }

    /// Returns if the client currently has consensus with the network.
    #[wasm_bindgen(js_name = isConsensusEstablished)]
    pub fn is_consensus_established(&self) -> bool {
        self.inner.consensus_proxy().is_established()
    }

    /// Returns a promise that resolves when the client has established consensus with the network.
    #[wasm_bindgen(js_name = waitForConsensusEstablished)]
    pub async fn wait_for_consensus_established(&self) -> Result<(), JsError> {
        if self.is_consensus_established() {
            return Ok(());
        }

        let is_established = self
            .inner
            .consensus_proxy()
            .subscribe_events()
            .any(|event| async move {
                if let Ok(state) = event {
                    matches!(state, ConsensusEvent::Established)
                } else {
                    self.is_consensus_established()
                }
            })
            .await;

        if !is_established {
            // The stream terminated before an `Established` event occured
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
        let address = Address::from_any(address)?;
        let plain_accounts = self.get_plain_accounts(vec![address]).await?;
        let account = plain_accounts
            .get(0)
            .ok_or_else(|| JsError::new("Could not get account"))?;
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
        // Unpack the array of addresses
        let js_value: &JsValue = addresses.unchecked_ref();
        let array: &Array = js_value
            .dyn_ref()
            .ok_or_else(|| JsError::new("`addresses` must be an array"))?;
        let mut addresses = Vec::<_>::with_capacity(array.length().try_into()?);
        for any in array.iter() {
            let address = Address::from_any(&any.into())?;
            addresses.push(address);
        }

        let plain_accounts = self.get_plain_accounts(addresses).await?;
        Ok(serde_wasm_bindgen::to_value(&plain_accounts)?.into())
    }

    /// Instantiates a transaction builder class that provides helper methods to create transactions.
    #[wasm_bindgen(js_name = transactionBuilder)]
    pub fn transaction_builder(&self) -> TransactionBuilder {
        TransactionBuilder::new(self.network_id, self.inner.blockchain())
    }

    /// Sends a transaction to the network and returns {@link PlainTransactionDetails}.
    ///
    /// Throws in case of a networking error.
    #[wasm_bindgen(js_name = sendTransaction)]
    pub async fn send_transaction(
        &self,
        transaction: &TransactionAnyType,
    ) -> Result<PlainTransactionDetailsType, JsError> {
        let tx = Transaction::from_any(transaction)?;

        tx.verify(Some(self.network_id))?;

        let current_height = self.get_head_height().await;

        self.inner
            .consensus_proxy()
            .send_transaction(tx.native())
            .await?;

        // Until we have a proper way of subscribing & listening for inclusion events of transactions,
        // we poll the sender's transaction receipts until we find the transaction's hash.
        // TODO: Instead of polling, subscribe to the transaction's inclusion events, or the sender's tx events.
        let tx_hash = tx.hash();
        let start = Date::now();

        loop {
            // Sleep for 0.5s before requesting (again)
            JsFuture::from(Promise::new(&mut |resolve, _| {
                web_sys::window()
                    .expect("Unable to get a reference to the JS `Window` object")
                    .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, 500)
                    .unwrap();
            }))
            .await
            .unwrap();

            let receipts = self
                .inner
                .consensus_proxy()
                .request_transaction_receipts_by_address(tx.sender().take_native(), 1, Some(10))
                .await?;

            for receipt in receipts {
                // The receipts are ordered newest first, so we can break the loop once receipts are older than
                // the blockchain height when we started to avoid looping over receipts that cannot be the one
                // we are looking for.
                if receipt.1 <= current_height {
                    break;
                }

                if receipt.0.to_hex() == tx_hash {
                    // Get the full transaction
                    let ext_tx = self
                        .inner
                        .consensus_proxy()
                        .request_transaction_by_hash_and_block_number(receipt.0, receipt.1, 1)
                        .await?;
                    let details =
                        PlainTransactionDetails::from_extended_transaction(&ext_tx, receipt.1);
                    return Ok(serde_wasm_bindgen::to_value(&details)?.into());
                }
            }

            if Date::now() - start >= 10_000.0 {
                break;
            }
        }

        // If the transaction did not get included, return it as TransactionState::New
        let details =
            PlainTransactionDetails::new(&tx, TransactionState::New, None, None, None, None);
        Ok(serde_wasm_bindgen::to_value(&details)?.into())
    }

    fn setup_offline_online_event_handlers(&self) {
        let window =
            web_sys::window().expect("Unable to get a reference to the JS `Window` object");
        let network = self.inner.network();
        let network1 = self.inner.network();

        // Register online closure
        let online_closure = Closure::<dyn Fn()>::new(move || {
            let network = network.clone();
            spawn_local(async move {
                let network = network.clone();
                network.restart_connecting().await;
            });
        });
        window
            .add_event_listener_with_callback("online", online_closure.as_ref().unchecked_ref())
            .expect("Unable to set callback for 'online' event");

        // Register offline closure
        let offline_closure = Closure::<dyn Fn()>::new(move || {
            let network = network1.clone();
            spawn_local(async move {
                let network = network.clone();
                network.disconnect(CloseReason::GoingOffline).await;
            });
        });
        window
            .add_event_listener_with_callback("offline", offline_closure.as_ref().unchecked_ref())
            .expect("Unable to set callback for 'offline' event");

        // Closures can't be dropped since they will be needed outside the context
        // of this function
        offline_closure.forget();
        online_closure.forget();
    }

    fn setup_consensus_events(&self) {
        let consensus = self.inner.consensus_proxy();
        let network = self.inner.network();

        let mut consensus_events = consensus.subscribe_events();

        let consensus_listeners = Rc::clone(&self.consensus_changed_listeners);

        spawn_local(async move {
            loop {
                let state = match consensus_events.next().await {
                    Some(Ok(ConsensusEvent::Established)) => Some(ConsensusState::Established),
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
        listeners: &Rc<RefCell<HashMap<usize, js_sys::Function>>>,
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

        let peer_listeners = Rc::clone(&self.peer_changed_listeners);
        let consensus_listeners = Rc::clone(&self.consensus_changed_listeners);

        spawn_local(async move {
            loop {
                let details = match network_events.next().await {
                    Some(Ok(NetworkEvent::PeerJoined(peer_id, peer_info))) => Some((
                        peer_id.to_string(),
                        "joined",
                        Some(PeerInfo::from_native(peer_info)),
                    )),
                    Some(Ok(NetworkEvent::PeerLeft(peer_id))) => {
                        Some((peer_id.to_string(), "left", None))
                    }
                    Some(Err(_error)) => {
                        None // Ignore stream errors
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

    fn next_listener_id(&mut self) -> usize {
        self.listener_id += 1;
        self.listener_id
    }

    /// This function is used to query the network for transaction receipts from and to a
    /// specific address, that have been included in the chain.
    ///
    /// The obtained receipts are _not_ verified before being returned.
    ///
    /// Up to a `limit` number of transaction receipts are returned from newest to oldest.
    /// If the network does not have at least `min_peers` to query, then an error is returned.
    #[wasm_bindgen(js_name = getTransactionReceiptsByAddress)]
    pub async fn get_transaction_receipts_by_address(
        &self,
        address: &AddressAnyType,
        limit: Option<u16>,
        min_peers: Option<usize>,
    ) -> Result<PlainTransactionReceiptArrayType, JsError> {
        if let Some(max) = limit {
            if max > MAX_TRANSACTIONS_BY_ADDRESS {
                return Err(JsError::new(
                    "The maximum number of transaction receipts exceeds the one that is supported",
                ));
            }
        }

        let receipts = self
            .inner
            .consensus_proxy()
            .request_transaction_receipts_by_address(
                Address::from_any(address)?.take_native(),
                min_peers.unwrap_or(1),
                limit,
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
    /// Up to a `limit` number of transactions are returned from newest to oldest.
    /// If the network does not have at least `min_peers` to query, then an error is returned.
    #[wasm_bindgen(js_name = getTransactionsByAddress)]
    pub async fn get_transactions_by_address(
        &self,
        address: &AddressAnyType,
        since_block_height: Option<u32>,
        known_transaction_details: Option<PlainTransactionDetailsArrayType>,
        limit: Option<u16>,
        min_peers: Option<usize>,
    ) -> Result<PlainTransactionDetailsArrayType, JsError> {
        if let Some(max) = limit {
            if max > MAX_TRANSACTIONS_BY_ADDRESS {
                return Err(JsError::new(
                    "The maximum number of transactions exceeds the one that is supported",
                ));
            }
        }

        let mut known_hashes = vec![];

        if let Some(array) = known_transaction_details {
            let plain_tx_details =
                serde_wasm_bindgen::from_value::<Vec<PlainTransactionDetails>>(array.into())?;
            for obj in plain_tx_details {
                match obj.state {
                    // Do not skip unconfirmed transactions
                    TransactionState::New
                    | TransactionState::Pending
                    | TransactionState::Included => continue,
                    _ => {
                        known_hashes.push(Blake2bHash::from_str(&obj.transaction.transaction_hash)?)
                    }
                }
            }
        }

        let transactions = self
            .inner
            .consensus_proxy()
            .request_transactions_by_address(
                Address::from_any(address)?.take_native(),
                since_block_height.unwrap_or(0),
                known_hashes,
                min_peers.unwrap_or(1),
                limit,
            )
            .await?;

        let current_height = self.get_head_height().await;

        let plain_tx_details: Vec<_> = transactions
            .into_iter()
            .map(|ext_tx| {
                PlainTransactionDetails::from_extended_transaction(&ext_tx, current_height)
            })
            .collect();

        Ok(serde_wasm_bindgen::to_value(&plain_tx_details)?.into())
    }
}

impl Client {
    pub async fn get_plain_accounts(
        &self,
        addresses: Vec<Address>,
    ) -> Result<Vec<PlainAccount>, JsError> {
        let native_addresses: Vec<nimiq_keys::Address> = addresses
            .into_iter()
            .map(|addr| addr.take_native())
            .collect();

        let accounts: HashMap<_, _> = self
            .inner
            .consensus_proxy()
            .request_accounts_by_addresses(native_addresses.clone(), 1)
            .await?
            .into_iter()
            .collect();

        let mut ordered_accounts = vec![];
        let default = nimiq_account::Account::default();

        for address in &native_addresses {
            if let Some(maybe_account) = accounts.get(address) {
                let account = maybe_account.as_ref().unwrap_or(&default);
                ordered_accounts.push(PlainAccount::from_native(account));
            }
        }

        Ok(ordered_accounts)
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
        typescript_type = "(peer_id: string, reason: 'joined' | 'left', peer_count: number, peer_info?: PeerInfo) => any"
    )]
    pub type PeerChangedListener;
}
