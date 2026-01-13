use std::{io::Write, path::PathBuf, str::FromStr, sync::Arc};

use anyhow::{Context, Result};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::Json,
    routing::get,
    Router,
};
use clap::{crate_authors, crate_description, crate_version, Arg, Command};
use log::{error, info, warn};
use nimiq_database::{
    declare_table,
    mdbx::MdbxDatabase,
    traits::{Database, ReadCursor, ReadTransaction, WriteTransaction},
};
use nimiq_database_value_derive::DbSerializable;
use nimiq_hash::{HashOutput, Hasher, Keccak256Hash, Keccak256Hasher, SerializeContent};
use nimiq_keys::{Address, KeyPair, PrivateKey};
use nimiq_primitives::{coin::Coin, networks::NetworkId};
use nimiq_serde::{Deserialize, Serialize};
use nimiq_transaction::{
    account::{
        htlc_contract::AnyHash,
        oracle_contract::{IncomingOracleTransactionData, OracleContractVerifier},
        AccountTransactionVerification,
    },
    SignatureProof, Transaction,
};
use nimiq_utils::merkle::{compute_root_from_content, MerklePath};
use serde::{Deserialize as SerdeDeserialize, Serialize as SerdeSerialize};
use serde_json::json;
use tokio::time::{sleep, Duration};

#[derive(Debug, Clone, SerdeSerialize, Deserialize)]
struct Config {
    /// Polygon RPC endpoint
    polygon_rpc_url: String,
    /// Bridge contract address on Polygon
    bridge_contract_address: String,
    /// Nimiq RPC endpoint for submitting transactions
    nimiq_rpc_url: Option<String>,
    /// Oracle contract address on Nimiq
    oracle_contract_address: String,
    /// Private key for signing Oracle update transactions (hex encoded)
    oracle_owner_private_key: String,
    /// Network ID for Nimiq (e.g., "main", "test", "unit")
    nimiq_network_id: String,
    /// Starting block number to monitor from (optional, defaults to latest)
    start_block: Option<u64>,
    /// Polling interval in seconds (default: 12, Polygon block time)
    polling_interval: Option<u64>,
    /// Database path for storing burn events
    database_path: String,
    /// API server port (default: 8080)
    api_port: Option<u16>,
}

/// Burn event signature: Burn(address indexed account, uint256 amount)
/// keccak256("Burn(address,uint256)") = 0xcc16f5dbb4873280815c1ee09dbd06736cffcc184412cf7a71a0fdb75d397ca5
const BURN_EVENT_SIGNATURE: &str =
    "0xcc16f5dbb4873280815c1ee09dbd06736cffcc184412cf7a71a0fdb75d397ca5";

// Database table declarations
declare_table!(BurnEventTable, "BurnEvents", u64 => BurnEventData);
declare_table!(MerkleRootTable, "MerkleRoot", () => Vec<u8>);

/// Stored burn event data
#[derive(Debug, Clone, Serialize, Deserialize, DbSerializable)]
pub struct BurnEventData {
    /// The account that burned the tokens
    account: Vec<u8>,
    /// The amount burned (32 bytes)
    amount: Vec<u8>,
    /// Transaction hash (32 bytes)
    tx_hash: Vec<u8>,
    /// Block number
    block_number: u64,
    /// Log index
    log_index: u64,
    /// The hash of this event (leaf in Merkle tree)
    event_hash: Keccak256Hash,
}

impl SerializeContent for BurnEventData {
    fn serialize_content<W: Write, H: HashOutput>(&self, writer: &mut W) -> std::io::Result<()> {
        // Serialize the event data that was used to create the hash
        writer.write_all(&self.account)?;
        writer.write_all(&self.amount)?;
        writer.write_all(&self.tx_hash)?;
        writer.write_all(&self.block_number.to_be_bytes())?;
        writer.write_all(&self.log_index.to_be_bytes())?;
        Ok(())
    }
}

impl BurnEventData {
    fn new(
        account: Vec<u8>,
        amount: Vec<u8>,
        tx_hash: Vec<u8>,
        block_number: u64,
        log_index: u64,
    ) -> Self {
        // Create hash of the burn event data
        let mut event_data = Vec::new();
        event_data.extend_from_slice(&account);
        event_data.extend_from_slice(&amount);
        event_data.extend_from_slice(&tx_hash);
        event_data.extend_from_slice(&block_number.to_be_bytes());
        event_data.extend_from_slice(&log_index.to_be_bytes());

        let event_hash = Keccak256Hasher::default().digest(&event_data);

        Self {
            account,
            amount,
            tx_hash,
            block_number,
            log_index,
            event_hash,
        }
    }
}

/// Application state shared between monitoring and API
struct AppState {
    db: MdbxDatabase,
    config: Config,
    key_pair: KeyPair,
    oracle_address: Address,
    network_id: NetworkId,
}

#[derive(Debug, SerdeDeserialize)]
#[allow(dead_code)]
struct JsonRpcResponse<T> {
    jsonrpc: String,
    id: u64,
    result: Option<T>,
    error: Option<JsonRpcError>,
}

#[derive(Debug, SerdeDeserialize)]
#[allow(dead_code)]
struct JsonRpcError {
    code: i32,
    message: String,
}

#[derive(Debug, SerdeDeserialize)]
#[allow(dead_code)]
struct LogEntry {
    address: String,
    topics: Vec<String>,
    data: String,
    block_number: String,
    transaction_hash: String,
    transaction_index: String,
    log_index: String,
    block_hash: String,
}

#[derive(Debug, SerdeSerialize)]
struct MerkleProofResponse {
    /// The burn event data
    event: BurnEventResponse,
    /// Merkle path (proof)
    merkle_path: Vec<MerklePathNode>,
    /// Current Merkle root stored in Oracle
    merkle_root: String,
}

#[derive(Debug, SerdeSerialize)]
struct BurnEventResponse {
    account: String,
    amount: String,
    tx_hash: String,
    block_number: u64,
    log_index: u64,
    event_hash: String,
}

#[derive(Debug, SerdeSerialize)]
struct MerklePathNode {
    hash: String,
    left: bool,
}

fn main() {
    let exit_code = match run_app() {
        Ok(_) => 0,
        Err(e) => {
            eprintln!("Error: {e}");
            1
        }
    };
    std::process::exit(exit_code);
}

fn run_app() -> Result<()> {
    // Initialize logging
    env_logger::Builder::from_default_env().init();

    let matches = Command::new("Polygon Bridge Monitor")
        .version(crate_version!())
        .author(crate_authors!())
        .about(crate_description!())
        .arg(
            Arg::new("config")
                .short('c')
                .long("config")
                .value_name("CONFIG_FILE")
                .help("Path to configuration file (TOML format)")
                .required(true),
        )
        .get_matches();

    let config_path = matches
        .get_one::<String>("config")
        .expect("config is required");
    let config_path = PathBuf::from(config_path);

    // Load configuration
    let config_content = std::fs::read_to_string(&config_path)
        .with_context(|| format!("Failed to read config file: {:?}", config_path))?;
    let config: Config =
        toml::from_str(&config_content).with_context(|| "Failed to parse config file as TOML")?;

    info!("Starting Polygon Bridge Monitor");
    info!("Bridge contract: {}", config.bridge_contract_address);
    info!("Oracle contract: {}", config.oracle_contract_address);

    // Run the monitor
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(monitor_bridge_events(config))
}

async fn monitor_bridge_events(config: Config) -> Result<()> {
    // Parse addresses and keys
    let bridge_address = config
        .bridge_contract_address
        .trim_start_matches("0x")
        .to_lowercase();
    if bridge_address.len() != 40 {
        return Err(anyhow::anyhow!("Invalid bridge contract address"));
    }

    let oracle_address = Address::from_user_friendly_address(&config.oracle_contract_address)
        .context("Invalid oracle contract address")?;

    let private_key_bytes = hex::decode(config.oracle_owner_private_key.trim_start_matches("0x"))
        .context("Invalid private key format")?;
    let private_key =
        PrivateKey::from_bytes(&private_key_bytes).context("Failed to deserialize private key")?;
    let key_pair: KeyPair = private_key.into();

    let network_id =
        NetworkId::from_str(&config.nimiq_network_id).context("Invalid Nimiq network ID")?;

    // Open database
    let db_path = PathBuf::from(&config.database_path);
    std::fs::create_dir_all(&db_path).context("Failed to create database directory")?;
    let db_config = nimiq_database::mdbx::DatabaseConfig::default();
    let db = MdbxDatabase::new(db_path, db_config)?;
    db.create_regular_table(&BurnEventTable);
    db.create_regular_table(&MerkleRootTable);

    // Create app state
    let app_state = Arc::new(AppState {
        db: db.clone(),
        config: config.clone(),
        key_pair: key_pair.clone(),
        oracle_address,
        network_id,
    });

    // Start API server
    let api_port = config.api_port.unwrap_or(8080);
    let api_state = Arc::clone(&app_state);
    tokio::spawn(async move {
        if let Err(e) = start_api_server(api_port, api_state).await {
            error!("API server error: {}", e);
        }
    });

    // Get starting block
    let mut current_block = if let Some(start_block) = config.start_block {
        start_block
    } else {
        get_latest_block_number(&config.polygon_rpc_url).await?
    };

    info!("Starting monitoring from block {}", current_block);
    info!("API server listening on port {}", api_port);

    let polling_interval = Duration::from_secs(config.polling_interval.unwrap_or(12));

    // Main monitoring loop
    loop {
        match process_blocks(&app_state, &bridge_address, &mut current_block).await {
            Ok(()) => {
                // Wait before checking next block
                sleep(polling_interval).await;
            }
            Err(e) => {
                error!("Error processing blocks: {}", e);
                // Wait a bit before retrying
                sleep(Duration::from_secs(5)).await;
            }
        }
    }
}

async fn start_api_server(port: u16, state: Arc<AppState>) -> Result<()> {
    let app = Router::new()
        .route("/proof/:tx_hash", get(get_merkle_proof))
        .route("/root", get(get_merkle_root))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{}", port)).await?;
    info!("API server started on port {}", port);
    axum::serve(listener, app).await?;
    Ok(())
}

async fn get_merkle_proof(
    State(state): State<Arc<AppState>>,
    Path(tx_hash): Path<String>,
) -> Result<Json<MerkleProofResponse>, (StatusCode, String)> {
    let tx_hash_bytes = hex::decode(tx_hash.trim_start_matches("0x"))
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("Invalid tx_hash: {}", e)))?;

    // Find the burn event and collect all events
    let txn = state.db.read_transaction();
    let cursor = txn.cursor(&BurnEventTable);
    let mut found_event: Option<BurnEventData> = None;
    let mut events: Vec<BurnEventData> = Vec::new();

    // Iterate through all events to find the one with matching tx_hash
    for (_, event) in cursor.into_iter_start() {
        if event.tx_hash == tx_hash_bytes {
            found_event = Some(event.clone());
        }
        events.push(event);
    }

    let event =
        found_event.ok_or_else(|| (StatusCode::NOT_FOUND, "Burn event not found".to_string()))?;

    if events.is_empty() {
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            "No events in database".to_string(),
        ));
    }

    // Compute Merkle path
    let merkle_path = MerklePath::new::<Keccak256Hasher, _>(&events, &event);
    let leaf_hash = event.event_hash;

    // Get current Merkle root
    let txn = state.db.read_transaction();
    let merkle_root_bytes = txn.get(&MerkleRootTable, &());
    let merkle_root = if let Some(bytes) = merkle_root_bytes {
        // Reconstruct hash from bytes
        if bytes.len() == 32 {
            let mut hash_bytes = [0u8; 32];
            hash_bytes.copy_from_slice(&bytes[..32]);
            // Keccak256Hash is a tuple struct, we need to construct it properly
            // For now, just recompute to ensure correctness
            compute_root_from_content::<Keccak256Hasher, _>(&events)
        } else {
            compute_root_from_content::<Keccak256Hasher, _>(&events)
        }
    } else {
        compute_root_from_content::<Keccak256Hasher, _>(&events)
    };

    // Convert Merkle path to response format
    // Serialize and deserialize to extract left/right info
    let path_bytes = merkle_path.serialize_to_vec();
    let deserialized_path: MerklePath<Keccak256Hash> =
        MerklePath::deserialize_from_vec(&path_bytes).map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to deserialize path: {}", e),
            )
        })?;

    // Extract path nodes with left/right info
    // Since MerklePathNode is private, we'll use the hashes and compute left/right from the path structure
    let path_hashes = deserialized_path.hashes();

    // We need to determine left/right by checking the tree structure
    // The MerklePath stores nodes in bottom-up order, and left=true means the node is on the left side
    // We can determine this by checking which subtree contains our leaf
    let event_index = events
        .iter()
        .position(|e| e.event_hash == leaf_hash)
        .ok_or_else(|| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Event not found in list".to_string(),
            )
        })?;

    // Build path nodes by traversing the tree structure
    let path_nodes = build_merkle_path_nodes_from_tree(&events, event_index, &path_hashes);

    Ok(Json(MerkleProofResponse {
        event: BurnEventResponse {
            account: hex::encode(&event.account),
            amount: hex::encode(&event.amount),
            tx_hash: hex::encode(&event.tx_hash),
            block_number: event.block_number,
            log_index: event.log_index,
            event_hash: hex::encode(leaf_hash.as_bytes()),
        },
        merkle_path: path_nodes,
        merkle_root: hex::encode(merkle_root.as_bytes()),
    }))
}

async fn get_merkle_root(
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let txn = state.db.read_transaction();
    let cursor = txn.cursor(&BurnEventTable);
    let events: Vec<BurnEventData> = cursor.into_iter_start().map(|(_, event)| event).collect();

    let merkle_root = if events.is_empty() {
        // Return zero hash if no events
        Keccak256Hasher::default().digest(&[])
    } else {
        compute_root_from_content::<Keccak256Hasher, _>(&events)
    };

    Ok(Json(json!({
        "merkle_root": hex::encode(merkle_root.as_bytes()),
        "event_count": events.len()
    })))
}

async fn get_latest_block_number(rpc_url: &str) -> Result<u64> {
    let client = reqwest::Client::new();
    let payload = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "eth_blockNumber",
        "params": []
    });

    let response = client
        .post(rpc_url)
        .json(&payload)
        .send()
        .await
        .context("Failed to send RPC request")?;

    let body = response
        .text()
        .await
        .context("Failed to read RPC response")?;

    let result: JsonRpcResponse<String> =
        serde_json::from_str(&body).context("Failed to parse RPC response")?;

    if let Some(error) = result.error {
        return Err(anyhow::anyhow!("RPC error: {}", error.message));
    }

    let block_hex = result.result.context("No result in RPC response")?;
    let block_number = u64::from_str_radix(block_hex.trim_start_matches("0x"), 16)
        .context("Failed to parse block number")?;

    Ok(block_number)
}

async fn process_blocks(
    state: &AppState,
    bridge_address: &str,
    current_block: &mut u64,
) -> Result<()> {
    // Get latest block
    let latest_block = get_latest_block_number(&state.config.polygon_rpc_url).await?;

    if *current_block > latest_block {
        // We're ahead, wait for new blocks
        return Ok(());
    }

    // Process blocks from current_block to latest_block
    let end_block = (*current_block + 1000).min(latest_block); // Process in batches

    info!("Processing blocks {} to {}", current_block, end_block);

    // Get logs for Burn events
    let logs = get_logs(
        &state.config.polygon_rpc_url,
        bridge_address,
        *current_block,
        end_block,
    )
    .await?;

    let mut new_events = false;
    for log in logs {
        match process_burn_event(state, &log).await {
            Ok(()) => {
                info!(
                    "Successfully processed burn event: {}",
                    log.transaction_hash
                );
                new_events = true;
            }
            Err(e) => {
                error!(
                    "Failed to process burn event {}: {}",
                    log.transaction_hash, e
                );
            }
        }
    }

    // If we have new events, update the Merkle tree and Oracle
    if new_events {
        update_merkle_tree_and_oracle(state).await?;
    }

    *current_block = end_block + 1;
    Ok(())
}

async fn get_logs(
    rpc_url: &str,
    bridge_address: &str,
    from_block: u64,
    to_block: u64,
) -> Result<Vec<LogEntry>> {
    let client = reqwest::Client::new();

    // Create filter for Burn events
    let filter = json!({
        "fromBlock": format!("0x{:x}", from_block),
        "toBlock": format!("0x{:x}", to_block),
        "address": format!("0x{}", bridge_address),
        "topics": [
            BURN_EVENT_SIGNATURE
        ]
    });

    let payload = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "eth_getLogs",
        "params": [filter]
    });

    let response = client
        .post(rpc_url)
        .json(&payload)
        .send()
        .await
        .context("Failed to send RPC request")?;

    let body = response
        .text()
        .await
        .context("Failed to read RPC response")?;

    let result: JsonRpcResponse<Vec<LogEntry>> =
        serde_json::from_str(&body).context("Failed to parse RPC response")?;

    if let Some(error) = result.error {
        return Err(anyhow::anyhow!("RPC error: {}", error.message));
    }

    Ok(result.result.unwrap_or_default())
}

async fn process_burn_event(state: &AppState, log: &LogEntry) -> Result<()> {
    // Parse the Burn event
    if log.topics.len() < 3 {
        return Err(anyhow::anyhow!("Invalid Burn event: insufficient topics"));
    }

    // Verify event signature
    if log.topics[0].to_lowercase() != BURN_EVENT_SIGNATURE.to_lowercase() {
        return Err(anyhow::anyhow!("Invalid event signature"));
    }

    let account = &log.topics[1];
    let amount = &log.topics[2];

    // Parse block number and log index
    let block_number = u64::from_str_radix(log.block_number.trim_start_matches("0x"), 16)
        .context("Failed to parse block number")?;

    let log_index = u64::from_str_radix(log.log_index.trim_start_matches("0x"), 16)
        .context("Failed to parse log index")?;

    // Parse account (remove 0x prefix and take last 20 bytes)
    let account_bytes =
        hex::decode(account.trim_start_matches("0x")).context("Failed to decode account")?;
    let account_final = account_bytes[account_bytes.len() - 20..].to_vec();

    // Parse amount (32 bytes)
    let amount_bytes =
        hex::decode(amount.trim_start_matches("0x")).context("Failed to decode amount")?;

    // Parse transaction hash (32 bytes)
    let tx_hash_bytes = hex::decode(log.transaction_hash.trim_start_matches("0x"))
        .context("Failed to decode transaction hash")?;

    // Check if event already exists
    let txn = state.db.read_transaction();
    let cursor = txn.cursor(&BurnEventTable);
    let mut max_index = 0u64;
    let mut exists = false;
    for (index, event) in cursor.into_iter_start() {
        max_index = max_index.max(index);
        if event.tx_hash == tx_hash_bytes && event.log_index == log_index {
            exists = true;
            break;
        }
    }

    if exists {
        info!("Burn event already exists: {}", log.transaction_hash);
        return Ok(());
    }

    // Create and store burn event
    let event_data = BurnEventData::new(
        account_final,
        amount_bytes,
        tx_hash_bytes,
        block_number,
        log_index,
    );

    let mut txn = state.db.write_transaction();
    txn.put(&BurnEventTable, &(max_index + 1), &event_data);
    txn.commit();

    info!(
        "Stored burn event - Account: {}, Amount: {}, Hash: {}",
        hex::encode(&event_data.account),
        hex::encode(&event_data.amount),
        hex::encode(event_data.event_hash.as_bytes())
    );

    Ok(())
}

async fn update_merkle_tree_and_oracle(state: &AppState) -> Result<()> {
    // Get all events from database
    let txn = state.db.read_transaction();
    let cursor = txn.cursor(&BurnEventTable);
    let events: Vec<BurnEventData> = cursor.into_iter_start().map(|(_, event)| event).collect();

    if events.is_empty() {
        warn!("No events to build Merkle tree from");
        return Ok(());
    }

    // Compute Merkle root
    let merkle_root = compute_root_from_content::<Keccak256Hasher, _>(&events);
    info!(
        "Computed Merkle root: {}",
        hex::encode(merkle_root.as_bytes())
    );

    // Check if root has changed
    let txn = state.db.read_transaction();
    let stored_root_bytes = txn.get(&MerkleRootTable, &());
    if let Some(stored_bytes) = stored_root_bytes {
        if stored_bytes.len() == 32 && stored_bytes == merkle_root.as_bytes() {
            info!("Merkle root unchanged, skipping Oracle update");
            return Ok(());
        }
    }

    // Store new root
    let mut txn = state.db.write_transaction();
    txn.put(&MerkleRootTable, &(), &merkle_root.as_bytes().to_vec());
    txn.commit();

    // Convert Merkle root to AnyHash
    let any_hash = AnyHash::from(merkle_root);

    // Create Oracle update transaction
    let tx = create_oracle_update_transaction(
        state.oracle_address.clone(),
        &state.key_pair,
        vec![any_hash],
        state.network_id,
        &state.config,
    )
    .await?;

    // Submit transaction to Nimiq
    if let Some(ref nimiq_rpc_url) = state.config.nimiq_rpc_url {
        submit_transaction_to_nimiq(nimiq_rpc_url, &tx).await?;
    } else {
        // If no RPC URL provided, just log the transaction
        info!(
            "Oracle update transaction created (not submitted): {}",
            hex::encode(tx.serialize_to_vec())
        );
    }

    Ok(())
}

async fn create_oracle_update_transaction(
    oracle_address: Address,
    key_pair: &KeyPair,
    hashes: Vec<AnyHash>,
    network_id: NetworkId,
    config: &Config,
) -> Result<Transaction> {
    // Get current block height from Nimiq RPC if available
    let validity_start_height = if let Some(ref nimiq_rpc_url) = config.nimiq_rpc_url {
        get_nimiq_block_height(nimiq_rpc_url).await.unwrap_or(0)
    } else {
        0
    };

    // Create update data without signature first
    let update_data = IncomingOracleTransactionData::Update {
        hashes,
        proof: SignatureProof::default(),
    };

    let mut tx = Transaction::new_signaling(
        oracle_address.clone(),
        nimiq_primitives::account::AccountType::Oracle,
        oracle_address,
        nimiq_primitives::account::AccountType::Oracle,
        Coin::ZERO, // Fee will be set by the network
        update_data.serialize_to_vec(),
        validity_start_height,
        network_id,
    );

    // Create signature proof by signing the transaction with the data (but without signature)
    let signature = key_pair.sign(&tx.serialize_content());
    let signature_proof = SignatureProof::from_ed25519(key_pair.public.clone(), signature);

    // Set the signature in the recipient_data
    tx.recipient_data =
        IncomingOracleTransactionData::set_signature_on_data(&tx.recipient_data, signature_proof)
            .context("Failed to set signature on data")?;

    // Verify the transaction
    OracleContractVerifier::verify_incoming_transaction(&tx)
        .context("Failed to verify Oracle transaction")?;

    Ok(tx)
}

async fn get_nimiq_block_height(rpc_url: &str) -> Result<u32> {
    let client = reqwest::Client::new();
    let payload = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "blockNumber",
        "params": []
    });

    let response = client
        .post(rpc_url)
        .json(&payload)
        .send()
        .await
        .context("Failed to send RPC request")?;

    let body = response
        .text()
        .await
        .context("Failed to read RPC response")?;

    let result: JsonRpcResponse<u32> =
        serde_json::from_str(&body).context("Failed to parse RPC response")?;

    if let Some(error) = result.error {
        return Err(anyhow::anyhow!("RPC error: {}", error.message));
    }

    result.result.context("No result in RPC response")
}

async fn submit_transaction_to_nimiq(rpc_url: &str, tx: &Transaction) -> Result<()> {
    // Serialize transaction
    let tx_bytes = tx.serialize_to_vec();
    let tx_hex = hex::encode(tx_bytes);

    // Submit via JSON-RPC
    let client = reqwest::Client::new();
    let payload = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "sendRawTransaction",
        "params": [tx_hex]
    });

    let response = client
        .post(rpc_url)
        .json(&payload)
        .send()
        .await
        .context("Failed to send RPC request")?;

    let status = response.status();
    let body = response
        .text()
        .await
        .context("Failed to read RPC response")?;

    if !status.is_success() {
        return Err(anyhow::anyhow!(
            "RPC request failed with status {}: {}",
            status,
            body
        ));
    }

    let result: JsonRpcResponse<String> =
        serde_json::from_str(&body).context("Failed to parse RPC response")?;

    if let Some(error) = result.error {
        return Err(anyhow::anyhow!("RPC error: {}", error.message));
    }

    info!("Transaction submitted successfully: {:?}", result.result);
    Ok(())
}

/// Build Merkle path nodes by traversing the tree structure
fn build_merkle_path_nodes_from_tree(
    events: &[BurnEventData],
    leaf_index: usize,
    path_hashes: &[Keccak256Hash],
) -> Vec<MerklePathNode> {
    let mut path_nodes = Vec::new();
    let mut current_index = leaf_index;
    let mut current_level_size = events.len();
    let mut path_hash_index = 0;

    // Traverse up the tree
    while current_level_size > 1 {
        let mid = current_level_size.div_ceil(2);
        let is_left = current_index < mid;

        if is_left {
            // We're on the left, so the sibling is on the right
            if path_hash_index < path_hashes.len() {
                path_nodes.push(MerklePathNode {
                    hash: hex::encode(path_hashes[path_hash_index].as_bytes()),
                    left: false, // Sibling is on the right
                });
                path_hash_index += 1;
            }
        } else {
            // We're on the right, so the sibling is on the left
            if path_hash_index < path_hashes.len() {
                path_nodes.push(MerklePathNode {
                    hash: hex::encode(path_hashes[path_hash_index].as_bytes()),
                    left: true, // Sibling is on the left
                });
                path_hash_index += 1;
            }
            current_index -= mid;
        }

        current_level_size = mid;
    }

    path_nodes
}

#[cfg(test)]
mod tests {
    use nimiq_utils::key_rng::SecureGenerate;

    use super::*;

    /// Helper function to create a test database
    fn create_test_db() -> MdbxDatabase {
        use tempfile::TempDir;
        let temp_dir = TempDir::new().unwrap();
        let db_config = nimiq_database::mdbx::DatabaseConfig::default();
        let db = MdbxDatabase::new(temp_dir.path(), db_config).unwrap();
        db.create_regular_table(&BurnEventTable);
        db.create_regular_table(&MerkleRootTable);
        // Keep temp_dir alive by leaking it (tests only)
        std::mem::forget(temp_dir);
        db
    }

    /// Helper function to create test burn events
    fn create_test_burn_event(
        account: &[u8],
        amount: &[u8],
        tx_hash: &[u8],
        block_number: u64,
        log_index: u64,
    ) -> BurnEventData {
        BurnEventData::new(
            account.to_vec(),
            amount.to_vec(),
            tx_hash.to_vec(),
            block_number,
            log_index,
        )
    }

    /// Helper function to create test app state
    fn create_test_app_state(db: MdbxDatabase) -> Arc<AppState> {
        let key_pair = KeyPair::generate_default_csprng();
        let oracle_address = Address::from([1u8; 20]);
        let config = Config {
            polygon_rpc_url: "https://polygon-rpc.com".to_string(),
            bridge_contract_address: "0x0000000000000000000000000000000000000000".to_string(),
            nimiq_rpc_url: None,
            oracle_contract_address: oracle_address.to_user_friendly_address(),
            oracle_owner_private_key: hex::encode(key_pair.private.serialize_to_vec()),
            nimiq_network_id: "test".to_string(),
            start_block: None,
            polling_interval: None,
            database_path: "".to_string(), // Not used in tests
            api_port: None,
        };

        Arc::new(AppState {
            db,
            config,
            key_pair,
            oracle_address,
            network_id: NetworkId::UnitAlbatross,
        })
    }

    #[test]
    fn test_burn_event_data_creation() {
        let account = vec![1u8; 20];
        let amount = vec![2u8; 32];
        let tx_hash = vec![3u8; 32];
        let block_number = 1000;
        let log_index = 5;

        let event = create_test_burn_event(&account, &amount, &tx_hash, block_number, log_index);

        assert_eq!(event.account, account);
        assert_eq!(event.amount, amount);
        assert_eq!(event.tx_hash, tx_hash);
        assert_eq!(event.block_number, block_number);
        assert_eq!(event.log_index, log_index);
        // Event hash should be computed
        assert_ne!(event.event_hash.as_bytes(), &[0u8; 32]);
    }

    #[test]
    fn test_merkle_tree_generation() {
        let db = create_test_db();
        let state = create_test_app_state(db);

        // Create multiple burn events
        let events = vec![
            create_test_burn_event(&[1u8; 20], &[10u8; 32], &[100u8; 32], 1000, 0),
            create_test_burn_event(&[2u8; 20], &[20u8; 32], &[200u8; 32], 1001, 1),
            create_test_burn_event(&[3u8; 20], &[30u8; 32], &[0x2Cu8; 32], 1002, 2),
            create_test_burn_event(&[4u8; 20], &[40u8; 32], &[0x90u8; 32], 1003, 3),
        ];

        // Store events in database
        let mut txn = state.db.write_transaction();
        for (i, event) in events.iter().enumerate() {
            txn.put(&BurnEventTable, &(i as u64 + 1), event);
        }
        txn.commit();

        // Compute Merkle root
        let merkle_root = compute_root_from_content::<Keccak256Hasher, _>(&events);

        // Store root
        let mut txn = state.db.write_transaction();
        txn.put(&MerkleRootTable, &(), &merkle_root.as_bytes().to_vec());
        txn.commit();

        // Verify root is stored correctly
        let txn = state.db.read_transaction();
        let stored_root = txn.get(&MerkleRootTable, &()).unwrap();
        assert_eq!(stored_root, merkle_root.as_bytes());

        // Add another event and verify root changes
        let new_event = create_test_burn_event(&[5u8; 20], &[50u8; 32], &[0xF4u8; 32], 1004, 4);
        let mut txn = state.db.write_transaction();
        txn.put(&BurnEventTable, &5, &new_event);
        txn.commit();

        let txn = state.db.read_transaction();
        let cursor = txn.cursor(&BurnEventTable);
        let all_events: Vec<BurnEventData> =
            cursor.into_iter_start().map(|(_, event)| event).collect();

        let new_root = compute_root_from_content::<Keccak256Hasher, _>(&all_events);
        assert_ne!(
            new_root, merkle_root,
            "Root should change when new event is added"
        );
    }

    #[tokio::test]
    async fn test_merkle_proof_api_endpoint() {
        let db = create_test_db();
        let state = create_test_app_state(db);

        // Create test events
        let events = vec![
            create_test_burn_event(&[1u8; 20], &[10u8; 32], &[100u8; 32], 1000, 0),
            create_test_burn_event(&[2u8; 20], &[20u8; 32], &[200u8; 32], 1001, 1),
            create_test_burn_event(&[3u8; 20], &[30u8; 32], &[0x2Cu8; 32], 1002, 2),
        ];

        // Store events
        let mut txn = state.db.write_transaction();
        for (i, event) in events.iter().enumerate() {
            txn.put(&BurnEventTable, &(i as u64 + 1), event);
        }
        txn.commit();

        // Test getting proof for first event
        let tx_hash = hex::encode(&[100u8; 32]);
        let result = get_merkle_proof(State(state.clone()), Path(tx_hash)).await;

        assert!(result.is_ok(), "Should successfully get Merkle proof");
        let proof_response = result.unwrap().0;

        // Verify proof structure
        assert_eq!(proof_response.event.tx_hash, hex::encode(&[100u8; 32]));
        assert_eq!(proof_response.event.block_number, 1000);
        assert_eq!(proof_response.event.log_index, 0);
        assert!(
            !proof_response.merkle_path.is_empty(),
            "Merkle path should not be empty"
        );
        assert!(
            !proof_response.merkle_root.is_empty(),
            "Merkle root should not be empty"
        );

        // Verify proof can be used to compute root
        let merkle_path = MerklePath::new::<Keccak256Hasher, _>(&events, &events[0]);
        let computed_root = merkle_path.compute_root(&events[0]);
        let expected_root = compute_root_from_content::<Keccak256Hasher, _>(&events);
        assert_eq!(
            computed_root, expected_root,
            "Merkle proof should compute correct root"
        );
    }

    #[tokio::test]
    async fn test_merkle_root_api_endpoint() {
        let db = create_test_db();
        let state = create_test_app_state(db);

        // Initially no events
        let result = get_merkle_root(State(state.clone())).await;
        assert!(result.is_ok());
        let response = result.unwrap().0;
        assert_eq!(response["event_count"], 0);

        // Add events
        let events = vec![
            create_test_burn_event(&[1u8; 20], &[10u8; 32], &[100u8; 32], 1000, 0),
            create_test_burn_event(&[2u8; 20], &[20u8; 32], &[200u8; 32], 1001, 1),
        ];

        let mut txn = state.db.write_transaction();
        for (i, event) in events.iter().enumerate() {
            txn.put(&BurnEventTable, &(i as u64 + 1), event);
        }
        txn.commit();

        // Get root
        let result = get_merkle_root(State(state.clone())).await;
        assert!(result.is_ok());
        let response = result.unwrap().0;
        assert_eq!(response["event_count"], 2);
        assert!(!response["merkle_root"].as_str().unwrap().is_empty());

        // Verify root matches computed root
        let computed_root = compute_root_from_content::<Keccak256Hasher, _>(&events);
        let root_hex = hex::encode(computed_root.as_bytes());
        assert_eq!(response["merkle_root"].as_str().unwrap(), &root_hex);
    }

    #[tokio::test]
    async fn test_merkle_proof_nonexistent_tx() {
        let db = create_test_db();
        let state = create_test_app_state(db);

        // Try to get proof for non-existent transaction
        let tx_hash = hex::encode(&[0xE7u8; 32]);
        let result = get_merkle_proof(State(state.clone()), Path(tx_hash)).await;

        assert!(result.is_err());
        let (status, message) = result.unwrap_err();
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert!(message.contains("not found"));
    }

    #[test]
    fn test_merkle_tree_consistency() {
        // Test that Merkle tree is consistent across different orderings
        // (Note: Our implementation uses the order from database, so order matters)
        let events1 = vec![
            create_test_burn_event(&[1u8; 20], &[10u8; 32], &[100u8; 32], 1000, 0),
            create_test_burn_event(&[2u8; 20], &[20u8; 32], &[200u8; 32], 1001, 1),
        ];

        let events2 = vec![
            create_test_burn_event(&[2u8; 20], &[20u8; 32], &[200u8; 32], 1001, 1),
            create_test_burn_event(&[1u8; 20], &[10u8; 32], &[100u8; 32], 1000, 0),
        ];

        let root1 = compute_root_from_content::<Keccak256Hasher, _>(&events1);
        let root2 = compute_root_from_content::<Keccak256Hasher, _>(&events2);

        // Roots should be different because order matters in our implementation
        assert_ne!(
            root1, root2,
            "Different orderings should produce different roots"
        );
    }

    #[tokio::test]
    async fn test_oracle_transaction_creation() {
        use nimiq_utils::key_rng::SecureGenerate;
        let key_pair = KeyPair::generate_default_csprng();
        let oracle_address = Address::from([1u8; 20]);
        let network_id = NetworkId::UnitAlbatross;

        let config = Config {
            polygon_rpc_url: "".to_string(),
            bridge_contract_address: "".to_string(),
            nimiq_rpc_url: None,
            oracle_contract_address: oracle_address.to_user_friendly_address(),
            oracle_owner_private_key: hex::encode(key_pair.private.serialize_to_vec()),
            nimiq_network_id: "test".to_string(),
            start_block: None,
            polling_interval: None,
            database_path: "".to_string(),
            api_port: None,
        };

        // Create a test Merkle root
        let test_hash = Keccak256Hasher::default().digest(b"test merkle root");
        let any_hash = AnyHash::from(test_hash);

        let tx = create_oracle_update_transaction(
            oracle_address,
            &key_pair,
            vec![any_hash],
            network_id,
            &config,
        )
        .await
        .expect("Should create transaction");

        // Verify transaction structure
        assert_eq!(
            tx.recipient_type,
            nimiq_primitives::account::AccountType::Oracle
        );
        assert!(tx
            .flags
            .contains(nimiq_transaction::TransactionFlags::SIGNALING));

        // Verify transaction can be verified
        let verification_result = OracleContractVerifier::verify_incoming_transaction(&tx);
        assert!(verification_result.is_ok(), "Transaction should be valid");
    }

    #[test]
    fn test_multiple_events_merkle_tree() {
        // Test with various numbers of events to ensure tree works correctly
        for num_events in [1, 2, 3, 4, 5, 8, 10] {
            let events: Vec<BurnEventData> = (0..num_events)
                .map(|i| {
                    create_test_burn_event(
                        &[(i as u8); 20],
                        &[(i as u8 * 10); 32],
                        &[(i as u8 * 25); 32], // Use 25 instead of 100 to avoid overflow
                        1000 + i as u64,
                        i,
                    )
                })
                .collect();

            let root = compute_root_from_content::<Keccak256Hasher, _>(&events);
            assert_ne!(
                root.as_bytes(),
                &[0u8; 32],
                "Root should not be zero for {} events",
                num_events
            );

            // Verify each event has a valid proof
            for event in &events {
                let merkle_path = MerklePath::new::<Keccak256Hasher, _>(&events, event);
                let computed_root = merkle_path.compute_root(event);
                assert_eq!(
                    computed_root, root,
                    "Proof for event should compute to correct root"
                );
            }
        }
    }
}
