use std::{cell::RefCell, collections::HashMap, rc::Rc, str::FromStr};

use futures::StreamExt;
use js_sys::{Array, Uint8Array};
use log::level_filters::LevelFilter;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::spawn_local;

pub use nimiq::{
    client::Consensus,
    config::command_line::CommandLine,
    config::config::ClientConfig,
    config::config_file::{ConfigFile, LogSettings, Seed, SyncMode},
    error::Error,
    extras::{panic::initialize_panic_reporting, web_logging::initialize_web_logging},
};

use beserial::{Deserialize, Serialize};
use nimiq_blockchain_interface::{AbstractBlockchain, BlockchainEvent};
use nimiq_consensus::ConsensusEvent;
use nimiq_hash::Blake2bHash;
use nimiq_network_interface::{
    network::{CloseReason, Network, NetworkEvent},
    Multiaddr,
};
use nimiq_primitives::policy::Policy;

use crate::address::Address;
use crate::peer_info::PeerInfo;
use crate::transaction::{
    PlainTransactionDetails, PlainTransactionDetailsArrayType, Transaction, TransactionState,
};
use crate::transaction_builder::TransactionBuilder;
use crate::utils::{from_network_id, to_network_id};

mod address;
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
pub const MAX_TRANSACTIONS_BY_ADDRESS: u16 = 128;

/// Use this to provide initialization-time configuration to the Client.
/// This is a simplified version of the configuration that is used for regular nodes,
/// since not all configuration knobs are available when running inside a browser.
#[wasm_bindgen]
pub struct ClientConfiguration {
    seed_nodes: Vec<String>,
    log_level: String,
}

impl Default for ClientConfiguration {
    fn default() -> Self {
        Self {
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

        let network_id = from_network_id(config.network_id);

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
            network_id,
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
    pub fn add_consensus_changed_listener(
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
    pub fn add_head_changed_listener(
        &mut self,
        listener: HeadChangedListner,
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
    pub fn add_peer_changed_listener(
        &mut self,
        listener: PeerChangedListner,
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
    pub fn remove_listener(&self, handle: usize) {
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
    pub fn get_head_hash(&self) -> String {
        self.inner.blockchain_head().hash().to_hex()
    }

    /// Returns the block number of the current blockchain head.
    #[wasm_bindgen(js_name = getHeadHeight)]
    pub fn get_head_height(&self) -> u32 {
        self.inner.blockchain_head().block_number()
    }

    // TODO: Return a Block struct
    /// Returns the current blockchain head block.
    /// Note that the web client is a light client and does not have block bodies, i.e. no transactions.
    #[wasm_bindgen(js_name = getHeadBlock)]
    pub fn get_head_block(&self) -> Vec<u8> {
        let block = self.inner.blockchain_head();
        block.header().serialize_to_vec()
    }

    // TODO: Return a Block struct
    /// Fetches a block by its hash.
    ///
    /// Throws if the client does not have the block.
    ///
    /// Fetching blocks from the network is not yet available.
    #[wasm_bindgen(js_name = getBlock)]
    pub async fn get_block(&self, hash: &str) -> Result<Uint8Array, JsError> {
        let hash = Blake2bHash::from_str(hash)?;
        let block = self
            .inner
            .consensus_proxy()
            .blockchain
            .read()
            .get_block(&hash, false)?;
        let serialized = block.header().serialize_to_vec();
        Ok(Uint8Array::from(serialized.as_slice()))
    }

    // TODO: Return a Block struct
    /// Fetches a block by its height (block number).
    ///
    /// Throws if the client does not have the block.
    ///
    /// Fetching blocks from the network is not yet available.
    #[wasm_bindgen(js_name = getBlockAt)]
    pub async fn get_block_at(&self, height: u32) -> Result<Uint8Array, JsError> {
        let block = self
            .inner
            .consensus_proxy()
            .blockchain
            .read()
            .get_block_at(height, false)?;
        let serialized = block.header().serialize_to_vec();
        Ok(Uint8Array::from(serialized.as_slice()))
    }

    /// Instantiates a transaction builder class that provides helper methods to create transactions.
    #[wasm_bindgen(js_name = transactionBuilder)]
    pub fn transaction_builder(&self) -> TransactionBuilder {
        TransactionBuilder::new(self.network_id, self.inner.blockchain())
    }

    /// Sends a transaction to the network. This method does not check if the
    /// transaction gets included into a block.
    ///
    /// Throws in case of a networking error.
    #[wasm_bindgen(js_name = sendTransaction)]
    pub async fn send_transaction(&self, transaction: &Transaction) -> Result<(), JsError> {
        transaction.verify(Some(self.network_id))?;

        self.inner
            .consensus_proxy()
            .send_transaction(transaction.native_ref().clone())
            .await?;

        Ok(())
    }

    /// Sends a serialized transaction to the network. This method does not check if the
    /// transaction gets included into a block.
    ///
    /// Throws when the transaction cannot be parsed from the byte array or in case of a networking error.
    #[wasm_bindgen(js_name = sendRawTransaction)]
    pub async fn send_raw_transaction(&self, raw_tx: &[u8]) -> Result<(), JsError> {
        let tx = nimiq_transaction::Transaction::deserialize_from_vec(raw_tx)?;

        tx.verify(to_network_id(self.network_id).ok().unwrap())?;

        self.inner.consensus_proxy().send_transaction(tx).await?;

        Ok(())
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
        let mut consensus_events = self.inner.consensus_proxy().subscribe_events();

        let listeners_rc = Rc::clone(&self.consensus_changed_listeners);
        let this = JsValue::null();

        spawn_local(async move {
            loop {
                let state = match consensus_events.next().await {
                    Some(Ok(ConsensusEvent::Established)) => Some("established"),
                    Some(Ok(ConsensusEvent::Lost)) => Some("connecting"),
                    Some(Err(_)) => {
                        None // Ignore stream errors
                    }
                    None => {
                        break;
                    }
                };

                if state.is_none() {
                    continue;
                }

                let state = JsValue::from(state.unwrap());
                for listener in listeners_rc.borrow().values() {
                    let _ = listener.call1(&this, &state);
                }
            }
        });
    }

    fn setup_blockchain_events(&self) {
        let blockchain = self.inner.consensus_proxy().blockchain;
        let mut blockchain_events = blockchain.read().notifier_as_stream();

        let listeners_rc = Rc::clone(&self.head_changed_listeners);

        let this = JsValue::null();

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

                for listener in listeners_rc.borrow().values() {
                    let _ = listener.apply(&this, &args);
                }
            }
        });
    }

    fn setup_network_events(&self) {
        let network = self.inner.network();
        let mut network_events = network.subscribe_events();

        let listeners_rc = Rc::clone(&self.peer_changed_listeners);
        let this = JsValue::null();

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

                if let Some((peer_id, reason, peer_info)) = details {
                    let args = Array::new();
                    args.push(&peer_id.into());
                    args.push(&reason.into());
                    args.push(&network.peer_count().into());
                    args.push(&peer_info.into());

                    for listener in listeners_rc.borrow().values() {
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

    /// This function is used to query the network for transactions from a specific address, that
    /// have been included in the chain.
    /// The obtained transactions are verified before being returned.
    /// Up to a max number of transactions are returned from newest to oldest.
    /// If the network does not have at least min_peers to query, then an error is returned
    #[wasm_bindgen(js_name = getTransactionsByAddress)]
    pub async fn get_transations_by_address(
        &self,
        address: Address,
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
                address.native(),
                since_block_height.unwrap_or(0),
                known_hashes,
                min_peers.unwrap_or(1),
                limit,
            )
            .await?;

        let current_block = self.get_head_height();
        let last_macro_block = Policy::last_macro_block(current_block);

        let plain_tx_details: Vec<PlainTransactionDetails> = transactions
            .into_iter()
            .map(|ext_tx| {
                PlainTransactionDetails::from_extended_transaction(
                    &ext_tx,
                    current_block,
                    last_macro_block,
                )
            })
            .collect();

        Ok(serde_wasm_bindgen::to_value(&plain_tx_details)?.into())
    }
}

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(typescript_type = "(state: string) => any")]
    pub type ConsensusChangedListener;

    #[wasm_bindgen(
        typescript_type = "(hash: string, reason: string, reverted_blocks: string[], adopted_blocks: string[]) => any"
    )]
    pub type HeadChangedListner;

    #[wasm_bindgen(
        typescript_type = "(peer_id: string, reason: 'joined' | 'left', peer_count: number, peer_info?: PeerInfo) => any"
    )]
    pub type PeerChangedListner;
}
