use std::sync::Arc;

use async_trait::async_trait;
use nimiq_account::{Account, BridgeContract, OracleContract};
use nimiq_blockchain_interface::AbstractBlockchain;
use nimiq_blockchain_proxy::BlockchainReadProxy;
use nimiq_bls::{KeyPair as BlsKeyPair, SecretKey as BlsSecretKey};
use nimiq_consensus::{consensus::consensus_proxy::ConsensusSyncStatus, ConsensusProxy};
use nimiq_hash::{Blake2bHash, Hash};
use nimiq_keys::{Address, Ed25519PublicKey, KeyPair, PrivateKey};
use nimiq_network_libp2p::Network;
use nimiq_primitives::{coin::Coin, networks::NetworkId};
use nimiq_rpc_interface::{
    consensus::ConsensusInterface,
    types::{RPCResult, Transaction as RPCTransaction, ValidityStartHeight},
};
use nimiq_serde::{Deserialize, Serialize};
use nimiq_transaction::{
    account::{
        bridge_contract::{AnyMerkleProof, OutgoingTransaction},
        htlc_contract::{AnyHash, PreImage},
    },
    SignatureProof, Transaction,
};
use nimiq_transaction_builder::TransactionBuilder;
use parking_lot::RwLock;

use crate::{error::Error, wallets::UnlockedWallets};

pub struct ConsensusDispatcher {
    consensus: ConsensusProxy<Network>,
    unlocked_wallets: Option<Arc<RwLock<UnlockedWallets>>>,
}

impl ConsensusDispatcher {
    pub fn new(
        consensus: ConsensusProxy<Network>,
        unlocked_wallets: Option<Arc<RwLock<UnlockedWallets>>>,
    ) -> Self {
        Self {
            consensus,
            unlocked_wallets,
        }
    }

    /// Tries to fetch the key pair for the wallet with the given address.
    fn get_wallet_keypair(&self, address: &Address) -> Result<KeyPair, Error> {
        Ok(self
            .unlocked_wallets
            .as_ref()
            .ok_or_else(|| Error::UnlockedWalletNotFound(address.clone()))?
            .read()
            .get(address)
            .ok_or_else(|| Error::UnlockedWalletNotFound(address.clone()))?
            .key_pair
            .clone())
    }

    /// Returns the network ID for our current blockchain.
    fn get_network_id(&self) -> NetworkId {
        self.consensus.blockchain.read().network_id()
    }

    /// Calculates the actual block number for the validity start height given the ValidityStartHeight
    /// struct.
    fn validity_start_height(&self, validity_start_height: ValidityStartHeight) -> u32 {
        validity_start_height.block_number(self.consensus.blockchain.read().block_number())
    }

    /// Checks a bridge release against the current state of the bridge and its oracle, so that a
    /// release that would fail is not broadcast: a failed release still costs the signer its fee.
    ///
    /// The check needs the full state, so it is skipped on other nodes.
    fn check_bridge_release(
        &self,
        bridge_address: &Address,
        recipient: &Address,
        value: Coin,
        burn_proof: &OutgoingTransaction,
    ) -> Result<(), Error> {
        let BlockchainReadProxy::Full(blockchain) = self.consensus.blockchain.read() else {
            return Ok(());
        };

        let bridge = match blockchain
            .get_account_if_complete(bridge_address)
            .ok_or(Error::NoConsensus)?
        {
            Account::Bridge(bridge) => bridge,
            _ => {
                return Err(Error::BridgeReleaseRejected(format!(
                    "{bridge_address} is not a bridge contract"
                )))
            }
        };
        let oracle = match blockchain
            .get_account_if_complete(&bridge.oracle_address)
            .ok_or(Error::NoConsensus)?
        {
            Account::Oracle(oracle) => Some(oracle),
            _ => None,
        };
        let nonce = {
            let data_store = blockchain.state.accounts.data_store(bridge_address);
            let db_txn = blockchain.read_transaction();
            bridge.get_nonce(&data_store.read(&db_txn), recipient)
        };

        verify_bridge_release(
            &bridge,
            oracle.as_ref(),
            nonce,
            recipient,
            value,
            burn_proof,
        )
        .map_err(Error::BridgeReleaseRejected)
    }
}

fn transaction_to_hex_string(transaction: &Transaction) -> String {
    hex::encode(transaction.serialize_to_vec())
}

/// Parses the hex-encoded parts of a bridge burn proof.
fn parse_burn_proof(
    burn_transaction_data: &str,
    merkle_proof: &str,
    oracle_state_index: u64,
) -> Result<OutgoingTransaction, Error> {
    let merkle_proof = AnyMerkleProof::deserialize_all(&hex::decode(merkle_proof)?)
        .map_err(|_| Error::InvalidArgument("Merkle Proof".to_string()))?;

    OutgoingTransaction::new(
        hex::decode(burn_transaction_data)?,
        merkle_proof,
        oracle_state_index,
    )
    .map_err(|error| Error::InvalidArgument(format!("Burn Transaction Data: {error}")))
}

/// Checks a bridge release against the bridge, its oracle and the last `nonce` released to
/// `recipient`. The checks mirror `BridgeContract::commit_outgoing_transaction`.
///
/// Returns the reason why the release would fail.
fn verify_bridge_release(
    bridge: &BridgeContract,
    oracle: Option<&OracleContract>,
    nonce: u64,
    recipient: &Address,
    value: Coin,
    burn_proof: &OutgoingTransaction,
) -> Result<(), String> {
    let burn = burn_proof
        .parse_burn_data(&bridge.chain_config)
        .map_err(|error| format!("invalid burn transaction: {error}"))?;
    if burn.amount != value {
        return Err(format!("the burned amount is {}, not {value}", burn.amount));
    }
    if &burn.target_address != recipient {
        return Err(format!(
            "the burn transaction pays {}, not {recipient}",
            burn.target_address
        ));
    }
    if burn.target_chain_id != bridge.source_chain_id {
        return Err(format!(
            "the burn transaction is for chain {}, but the bridge serves chain {}",
            burn.target_chain_id, bridge.source_chain_id
        ));
    }
    if Some(burn.target_nonce) != nonce.checked_add(1) {
        return Err(format!(
            "the burn transaction has nonce {}, but the last nonce released to {recipient} is {nonce}",
            burn.target_nonce
        ));
    }
    if !burn_proof.is_proof_depth_valid(bridge.chain_config.max_proof_depth) {
        return Err(format!(
            "the Merkle proof is deeper than the maximum of {}",
            bridge.chain_config.max_proof_depth
        ));
    }

    // The proof is verified against the oracle state at `index`, chained onto the state before it
    // (or onto a zero hash for the very first state).
    let oracle = oracle.ok_or_else(|| {
        format!(
            "the bridge's oracle {} does not exist",
            bridge.oracle_address
        )
    })?;
    let index = burn_proof.oracle_state_index;
    let unavailable = |index: u64| format!("oracle state {index} is not available");
    let state = oracle
        .get_hash_at_index(index)
        .ok_or_else(|| unavailable(index))?;
    let leaf = burn_proof
        .extract_burn_transaction_hash(&bridge.chain_config.hash_function)
        .map_err(|error| format!("cannot hash the burn transaction: {error}"))?;
    let previous_state = match index.checked_sub(1) {
        Some(previous) => oracle
            .get_hash_at_index(previous)
            .cloned()
            .ok_or_else(|| unavailable(previous))?,
        None => leaf.zero_of_same_type(),
    };
    let root = burn_proof
        .compute_merkle_root(leaf)
        .map_err(|error| format!("invalid Merkle proof: {error}"))?;
    if *state != previous_state.digest(&root) {
        return Err(format!(
            "the Merkle proof does not match oracle state {index}"
        ));
    }

    if bridge.balance < value {
        return Err(format!("the bridge only holds {}", bridge.balance));
    }

    Ok(())
}

#[nimiq_jsonrpc_derive::service(rename_all = "camelCase")]
#[async_trait]
impl ConsensusInterface for ConsensusDispatcher {
    type Error = Error;

    async fn is_consensus_established(&self) -> RPCResult<bool, (), Self::Error> {
        Ok(self.consensus.is_established().into())
    }

    async fn get_sync_status(&self) -> RPCResult<ConsensusSyncStatus, (), Self::Error> {
        Ok(self.consensus.get_sync_status().into())
    }

    async fn get_raw_transaction_info(
        &self,
        raw_tx: String,
    ) -> RPCResult<RPCTransaction, (), Self::Error> {
        let transaction = Transaction::deserialize_from_vec(&hex::decode(raw_tx)?)?;
        Ok(RPCTransaction::from_transaction(transaction).into())
    }

    async fn send_raw_transaction(
        &self,
        raw_tx: String,
    ) -> RPCResult<Blake2bHash, (), Self::Error> {
        let tx = Transaction::deserialize_from_vec(&hex::decode(&raw_tx)?)?;
        let txid = tx.hash::<Blake2bHash>();

        match self.consensus.send_transaction(tx).await {
            Ok(_) => Ok(txid.into()),
            Err(e) => Err(Error::NetworkError(e)),
        }
    }

    async fn create_basic_transaction(
        &self,
        wallet: Address,
        recipient: Address,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<String, (), Self::Error> {
        let transaction = TransactionBuilder::new_basic(
            &self.get_wallet_keypair(&wallet)?,
            recipient,
            value,
            fee,
            self.validity_start_height(validity_start_height),
            self.get_network_id(),
        )?;

        Ok(transaction_to_hex_string(&transaction).into())
    }

    async fn send_basic_transaction(
        &self,
        wallet: Address,
        recipient: Address,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<Blake2bHash, (), Self::Error> {
        let raw_tx = self
            .create_basic_transaction(wallet, recipient, value, fee, validity_start_height)
            .await?
            .data;
        self.send_raw_transaction(raw_tx).await
    }

    async fn create_basic_transaction_with_data(
        &self,
        wallet: Address,
        recipient: Address,
        data: String,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<String, (), Self::Error> {
        let transaction = TransactionBuilder::new_basic_with_data(
            &self.get_wallet_keypair(&wallet)?,
            recipient,
            hex::decode(data)?,
            value,
            fee,
            self.validity_start_height(validity_start_height),
            self.get_network_id(),
        )?;

        Ok(transaction_to_hex_string(&transaction).into())
    }

    async fn send_basic_transaction_with_data(
        &self,
        wallet: Address,
        recipient: Address,
        data: String,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<Blake2bHash, (), Self::Error> {
        let raw_tx = self
            .create_basic_transaction_with_data(
                wallet,
                recipient,
                data,
                value,
                fee,
                validity_start_height,
            )
            .await?
            .data;
        self.send_raw_transaction(raw_tx).await
    }

    async fn create_new_vesting_transaction(
        &self,
        wallet: Address,
        owner: Address,
        start_time: u64,
        time_step: u64,
        num_steps: u32,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<String, (), Self::Error> {
        let transaction = TransactionBuilder::new_create_vesting(
            &self.get_wallet_keypair(&wallet)?,
            owner,
            start_time,
            time_step,
            num_steps,
            value,
            fee,
            self.validity_start_height(validity_start_height),
            self.get_network_id(),
        )?;

        Ok(transaction_to_hex_string(&transaction).into())
    }

    async fn send_new_vesting_transaction(
        &self,
        wallet: Address,
        owner: Address,
        start_time: u64,
        time_step: u64,
        num_steps: u32,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<Blake2bHash, (), Self::Error> {
        let raw_tx = self
            .create_new_vesting_transaction(
                wallet,
                owner,
                start_time,
                time_step,
                num_steps,
                value,
                fee,
                validity_start_height,
            )
            .await?
            .data;
        self.send_raw_transaction(raw_tx).await
    }

    async fn create_redeem_vesting_transaction(
        &self,
        wallet: Address,
        contract_address: Address,
        recipient: Address,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<String, (), Self::Error> {
        let transaction = TransactionBuilder::new_redeem_vesting(
            &self.get_wallet_keypair(&wallet)?,
            contract_address,
            recipient,
            value,
            fee,
            self.validity_start_height(validity_start_height),
            self.get_network_id(),
        )?;

        Ok(transaction_to_hex_string(&transaction).into())
    }

    async fn send_redeem_vesting_transaction(
        &self,
        wallet: Address,
        contract_address: Address,
        recipient: Address,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<Blake2bHash, (), Self::Error> {
        let raw_tx = self
            .create_redeem_vesting_transaction(
                wallet,
                contract_address,
                recipient,
                value,
                fee,
                validity_start_height,
            )
            .await?
            .data;
        self.send_raw_transaction(raw_tx).await
    }

    async fn create_new_htlc_transaction(
        &self,
        wallet: Address,
        htlc_sender: Address,
        htlc_recipient: Address,
        hash_root: AnyHash,
        hash_count: u8,
        timeout: u64,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<String, (), Self::Error> {
        let transaction = TransactionBuilder::new_create_htlc(
            &self.get_wallet_keypair(&wallet)?,
            htlc_sender,
            htlc_recipient,
            hash_root,
            hash_count,
            timeout,
            value,
            fee,
            self.validity_start_height(validity_start_height),
            self.get_network_id(),
        )?;

        Ok(transaction_to_hex_string(&transaction).into())
    }

    async fn send_new_htlc_transaction(
        &self,
        wallet: Address,
        htlc_sender: Address,
        htlc_recipient: Address,
        hash_root: AnyHash,
        hash_count: u8,
        timeout: u64,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<Blake2bHash, (), Self::Error> {
        let raw_tx = self
            .create_new_htlc_transaction(
                wallet,
                htlc_sender,
                htlc_recipient,
                hash_root,
                hash_count,
                timeout,
                value,
                fee,
                validity_start_height,
            )
            .await?
            .data;
        self.send_raw_transaction(raw_tx).await
    }

    async fn create_redeem_regular_htlc_transaction(
        &self,
        wallet: Address,
        contract_address: Address,
        recipient: Address,
        pre_image: PreImage,
        hash_root: AnyHash,
        hash_count: u8,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<String, (), Self::Error> {
        let transaction = TransactionBuilder::new_redeem_htlc_regular(
            &self.get_wallet_keypair(&wallet)?,
            contract_address,
            recipient,
            pre_image,
            hash_root,
            hash_count,
            value,
            fee,
            self.validity_start_height(validity_start_height),
            self.get_network_id(),
        )?;

        Ok(transaction_to_hex_string(&transaction).into())
    }

    async fn send_redeem_regular_htlc_transaction(
        &self,
        wallet: Address,
        contract_address: Address,
        recipient: Address,
        pre_image: PreImage,
        hash_root: AnyHash,
        hash_count: u8,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<Blake2bHash, (), Self::Error> {
        let raw_tx = self
            .create_redeem_regular_htlc_transaction(
                wallet,
                contract_address,
                recipient,
                pre_image,
                hash_root,
                hash_count,
                value,
                fee,
                validity_start_height,
            )
            .await?
            .data;
        self.send_raw_transaction(raw_tx).await
    }

    async fn create_redeem_timeout_htlc_transaction(
        &self,
        wallet: Address,
        contract_address: Address,
        recipient: Address,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<String, (), Self::Error> {
        let transaction = TransactionBuilder::new_redeem_htlc_timeout(
            &self.get_wallet_keypair(&wallet)?,
            contract_address,
            recipient,
            value,
            fee,
            self.validity_start_height(validity_start_height),
            self.get_network_id(),
        )?;

        Ok(transaction_to_hex_string(&transaction).into())
    }

    async fn send_redeem_timeout_htlc_transaction(
        &self,
        wallet: Address,
        contract_address: Address,
        recipient: Address,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<Blake2bHash, (), Self::Error> {
        let raw_tx = self
            .create_redeem_timeout_htlc_transaction(
                wallet,
                contract_address,
                recipient,
                value,
                fee,
                validity_start_height,
            )
            .await?
            .data;
        self.send_raw_transaction(raw_tx).await
    }

    async fn create_redeem_early_htlc_transaction(
        &self,
        contract_address: Address,
        recipient: Address,
        htlc_sender_signature: String,
        htlc_recipient_signature: String,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<String, (), Self::Error> {
        let sig_sender = SignatureProof::deserialize_from_vec(&hex::decode(htlc_sender_signature)?)
            .map_err(|_| Error::InvalidArgument("Signing Key".to_string()))?;

        let sig_recipient =
            SignatureProof::deserialize_from_vec(&hex::decode(htlc_recipient_signature)?)
                .map_err(|_| Error::InvalidArgument("Recipient Key".to_string()))?;

        let transaction = TransactionBuilder::new_redeem_htlc_early(
            contract_address,
            recipient,
            sig_sender,
            sig_recipient,
            value,
            fee,
            self.validity_start_height(validity_start_height),
            self.get_network_id(),
        )?;

        Ok(transaction_to_hex_string(&transaction).into())
    }

    async fn send_redeem_early_htlc_transaction(
        &self,
        contract_address: Address,
        recipient: Address,
        htlc_sender_signature: String,
        htlc_recipient_signature: String,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<Blake2bHash, (), Self::Error> {
        let raw_tx = self
            .create_redeem_early_htlc_transaction(
                contract_address,
                recipient,
                htlc_sender_signature,
                htlc_recipient_signature,
                value,
                fee,
                validity_start_height,
            )
            .await?
            .data;
        self.send_raw_transaction(raw_tx).await
    }

    async fn sign_redeem_early_htlc_transaction(
        &self,
        wallet: Address,
        contract_address: Address,
        recipient: Address,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<String, (), Self::Error> {
        let sig = TransactionBuilder::sign_htlc_early(
            &self.get_wallet_keypair(&wallet)?,
            contract_address,
            recipient,
            value,
            fee,
            self.validity_start_height(validity_start_height),
            self.get_network_id(),
        )?;

        Ok(hex::encode(sig.serialize_to_vec()).into())
    }

    async fn create_new_staker_transaction(
        &self,
        sender_wallet: Address,
        staker_wallet: Address,
        delegation: Option<Address>,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<String, (), Self::Error> {
        let transaction = TransactionBuilder::new_create_staker(
            &self.get_wallet_keypair(&sender_wallet)?,
            &self.get_wallet_keypair(&staker_wallet)?,
            delegation,
            value,
            fee,
            self.validity_start_height(validity_start_height),
            self.get_network_id(),
        )?;

        Ok(transaction_to_hex_string(&transaction).into())
    }

    async fn send_new_staker_transaction(
        &self,
        sender_wallet: Address,
        staker_wallet: Address,
        delegation: Option<Address>,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<Blake2bHash, (), Self::Error> {
        let raw_tx = self
            .create_new_staker_transaction(
                sender_wallet,
                staker_wallet,
                delegation,
                value,
                fee,
                validity_start_height,
            )
            .await?
            .data;
        self.send_raw_transaction(raw_tx).await
    }

    async fn create_stake_transaction(
        &self,
        sender_wallet: Address,
        staker_address: Address,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<String, (), Self::Error> {
        let transaction = TransactionBuilder::new_add_stake(
            &self.get_wallet_keypair(&sender_wallet)?,
            staker_address,
            value,
            fee,
            self.validity_start_height(validity_start_height),
            self.get_network_id(),
        )?;

        Ok(transaction_to_hex_string(&transaction).into())
    }

    async fn send_stake_transaction(
        &self,
        sender_wallet: Address,
        staker_address: Address,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<Blake2bHash, (), Self::Error> {
        let raw_tx = self
            .create_stake_transaction(
                sender_wallet,
                staker_address,
                value,
                fee,
                validity_start_height,
            )
            .await?
            .data;

        self.send_raw_transaction(raw_tx).await
    }

    async fn create_update_staker_transaction(
        &self,
        sender_wallet: Option<Address>,
        staker_wallet: Address,
        new_delegation: Option<Address>,
        reactivate_all_stake: bool,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<String, (), Self::Error> {
        let sender_key = match sender_wallet {
            None => None,
            Some(address) => Some(self.get_wallet_keypair(&address)?),
        };

        let transaction = TransactionBuilder::new_update_staker(
            sender_key.as_ref(),
            &self.get_wallet_keypair(&staker_wallet)?,
            new_delegation,
            reactivate_all_stake,
            fee,
            self.validity_start_height(validity_start_height),
            self.get_network_id(),
        )?;

        Ok(transaction_to_hex_string(&transaction).into())
    }

    async fn send_update_staker_transaction(
        &self,
        sender_wallet: Option<Address>,
        staker_wallet: Address,
        new_delegation: Option<Address>,
        reactivate_all_stake: bool,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<Blake2bHash, (), Self::Error> {
        let raw_tx = self
            .create_update_staker_transaction(
                sender_wallet,
                staker_wallet,
                new_delegation,
                reactivate_all_stake,
                fee,
                validity_start_height,
            )
            .await?
            .data;
        self.send_raw_transaction(raw_tx).await
    }

    async fn create_set_active_stake_transaction(
        &self,
        sender_wallet: Option<Address>,
        staker_wallet: Address,
        new_active_balance: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<String, (), Self::Error> {
        let sender_key = match sender_wallet {
            None => None,
            Some(address) => Some(self.get_wallet_keypair(&address)?),
        };

        let transaction = TransactionBuilder::new_set_active_stake(
            sender_key.as_ref(),
            &self.get_wallet_keypair(&staker_wallet)?,
            new_active_balance,
            fee,
            self.validity_start_height(validity_start_height),
            self.get_network_id(),
        )?;

        Ok(transaction_to_hex_string(&transaction).into())
    }

    async fn send_set_active_stake_transaction(
        &self,
        sender_wallet: Option<Address>,
        staker_wallet: Address,
        new_active_balance: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<Blake2bHash, (), Self::Error> {
        let raw_tx = self
            .create_set_active_stake_transaction(
                sender_wallet,
                staker_wallet,
                new_active_balance,
                fee,
                validity_start_height,
            )
            .await?
            .data;
        self.send_raw_transaction(raw_tx).await
    }

    async fn create_retire_stake_transaction(
        &self,
        sender_wallet: Option<Address>,
        staker_wallet: Address,
        retire_stake: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<String, (), Self::Error> {
        let sender_key = match sender_wallet {
            None => None,
            Some(address) => Some(self.get_wallet_keypair(&address)?),
        };

        let transaction = TransactionBuilder::new_retire_stake(
            sender_key.as_ref(),
            &self.get_wallet_keypair(&staker_wallet)?,
            retire_stake,
            fee,
            self.validity_start_height(validity_start_height),
            self.get_network_id(),
        )?;

        Ok(transaction_to_hex_string(&transaction).into())
    }

    async fn send_retire_stake_transaction(
        &self,
        sender_wallet: Option<Address>,
        staker_wallet: Address,
        retire_stake: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<Blake2bHash, (), Self::Error> {
        let raw_tx = self
            .create_retire_stake_transaction(
                sender_wallet,
                staker_wallet,
                retire_stake,
                fee,
                validity_start_height,
            )
            .await?
            .data;
        self.send_raw_transaction(raw_tx).await
    }

    async fn create_remove_stake_transaction(
        &self,
        staker_wallet: Address,
        recipient: Address,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<String, (), Self::Error> {
        let transaction = TransactionBuilder::new_remove_stake(
            &self.get_wallet_keypair(&staker_wallet)?,
            recipient,
            value,
            fee,
            self.validity_start_height(validity_start_height),
            self.get_network_id(),
        )?;

        Ok(transaction_to_hex_string(&transaction).into())
    }

    async fn send_remove_stake_transaction(
        &self,
        staker_wallet: Address,
        recipient: Address,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<Blake2bHash, (), Self::Error> {
        let raw_tx = self
            .create_remove_stake_transaction(
                staker_wallet,
                recipient,
                value,
                fee,
                validity_start_height,
            )
            .await?
            .data;
        self.send_raw_transaction(raw_tx).await
    }

    async fn create_new_validator_transaction(
        &self,
        sender_wallet: Address,
        validator_wallet: Address,
        signing_secret_key: String,
        voting_secret_key: String,
        reward_address: Address,
        signal_data: String,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<String, (), Self::Error> {
        let voting_secret_key =
            BlsSecretKey::deserialize_from_vec(&hex::decode(voting_secret_key)?)
                .map_err(|_| Error::InvalidArgument("Voting Key".to_string()))?;
        let hot_keypair = BlsKeyPair::from(voting_secret_key);

        let signing_secret_key =
            PrivateKey::deserialize_from_vec(&hex::decode(signing_secret_key)?)
                .map_err(|_| Error::InvalidArgument("Signing Key".to_string()))?;
        let signing_key = Ed25519PublicKey::from(&signing_secret_key);

        // Since JSON doesn't have a primitive for Option (it just has the null primitive), we can't
        // have a double Option. This becomes an issue when creating an update_validator transaction.
        // Instead we use the following work-around. We define the empty String to be None. So, in
        // this situation we have:
        // "" = None
        // "0x29a4b..." = Some(hash)
        let signal_data: Option<Blake2bHash> = if signal_data.is_empty() {
            None
        } else {
            Some(
                Blake2bHash::deserialize_from_vec(&hex::decode(signal_data)?)
                    .map_err(|_| Error::InvalidArgument("Signal Data".to_string()))?,
            )
        };

        let transaction = TransactionBuilder::new_create_validator(
            &self.get_wallet_keypair(&sender_wallet)?,
            &self.get_wallet_keypair(&validator_wallet)?,
            signing_key,
            &hot_keypair,
            reward_address,
            signal_data,
            fee,
            self.validity_start_height(validity_start_height),
            self.get_network_id(),
        )?;

        Ok(transaction_to_hex_string(&transaction).into())
    }

    async fn send_new_validator_transaction(
        &self,
        sender_wallet: Address,
        validator_wallet: Address,
        signing_secret_key: String,
        voting_secret_key: String,
        reward_address: Address,
        signal_data: String,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<Blake2bHash, (), Self::Error> {
        let raw_tx = self
            .create_new_validator_transaction(
                sender_wallet,
                validator_wallet,
                signing_secret_key,
                voting_secret_key,
                reward_address,
                signal_data,
                fee,
                validity_start_height,
            )
            .await?
            .data;
        self.send_raw_transaction(raw_tx).await
    }

    async fn create_update_validator_transaction(
        &self,
        sender_wallet: Address,
        validator_wallet: Address,
        new_signing_secret_key: Option<String>,
        new_voting_secret_key: Option<String>,
        new_reward_address: Option<Address>,
        new_signal_data: Option<String>,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<String, (), Self::Error> {
        let new_voting_keypair = match new_voting_secret_key {
            Some(key) => {
                let new_secret_key = BlsSecretKey::deserialize_from_vec(&hex::decode(key)?)
                    .map_err(|_| Error::InvalidArgument("Voting Key".to_string()))?;
                Some(BlsKeyPair::from(new_secret_key))
            }
            _ => None,
        };

        let new_signing_key = match new_signing_secret_key {
            Some(key) => {
                let secret_key = PrivateKey::deserialize_from_vec(&hex::decode(key)?)
                    .map_err(|_| Error::InvalidArgument("Signing Key".to_string()))?;
                Some(Ed25519PublicKey::from(&secret_key))
            }
            _ => None,
        };

        // Since JSON doesn't have a primitive for Option (it just has the null primitive), we can't
        // have a double Option. Instead we use the following work-around. We define the empty String
        // to be None. So, in this situation we have:
        // null = None
        // "" = Some(None)
        // "0x29a4b..." = Some(Some(hash))
        let new_signal_data: Option<Option<Blake2bHash>> = match new_signal_data {
            None => None,
            Some(string) => {
                if string.is_empty() {
                    Some(None)
                } else {
                    Some(Some(
                        Blake2bHash::deserialize_from_vec(&hex::decode(string)?)
                            .map_err(|_| Error::InvalidArgument("Signal Data".to_string()))?,
                    ))
                }
            }
        };

        let transaction = TransactionBuilder::new_update_validator(
            &self.get_wallet_keypair(&sender_wallet)?,
            &self.get_wallet_keypair(&validator_wallet)?,
            new_signing_key,
            new_voting_keypair.as_ref(),
            new_reward_address,
            new_signal_data,
            fee,
            self.validity_start_height(validity_start_height),
            self.get_network_id(),
        );

        Ok(transaction_to_hex_string(&transaction).into())
    }

    async fn send_update_validator_transaction(
        &self,
        sender_wallet: Address,
        validator_wallet: Address,
        new_signing_secret_key: Option<String>,
        new_voting_secret_key: Option<String>,
        new_reward_address: Option<Address>,
        new_signal_data: Option<String>,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<Blake2bHash, (), Self::Error> {
        let raw_tx = self
            .create_update_validator_transaction(
                sender_wallet,
                validator_wallet,
                new_signing_secret_key,
                new_voting_secret_key,
                new_reward_address,
                new_signal_data,
                fee,
                validity_start_height,
            )
            .await?
            .data;
        self.send_raw_transaction(raw_tx).await
    }

    async fn create_deactivate_validator_transaction(
        &self,
        sender_wallet: Address,
        validator_address: Address,
        signing_secret_key: String,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<String, (), Self::Error> {
        let secret_key = PrivateKey::deserialize_from_vec(&hex::decode(signing_secret_key)?)
            .map_err(|_| Error::InvalidArgument("Signing Key".to_string()))?;

        let signing_key_pair = KeyPair::from(secret_key);

        let transaction = TransactionBuilder::new_deactivate_validator(
            &self.get_wallet_keypair(&sender_wallet)?,
            validator_address,
            &signing_key_pair,
            fee,
            self.validity_start_height(validity_start_height),
            self.get_network_id(),
        );

        Ok(transaction_to_hex_string(&transaction).into())
    }

    async fn send_deactivate_validator_transaction(
        &self,
        sender_wallet: Address,
        validator_address: Address,
        signing_secret_key: String,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<Blake2bHash, (), Self::Error> {
        let raw_tx = self
            .create_deactivate_validator_transaction(
                sender_wallet,
                validator_address,
                signing_secret_key,
                fee,
                validity_start_height,
            )
            .await?
            .data;
        self.send_raw_transaction(raw_tx).await
    }

    async fn create_reactivate_validator_transaction(
        &self,
        sender_wallet: Address,
        validator_address: Address,
        signing_secret_key: String,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<String, (), Self::Error> {
        let secret_key = PrivateKey::deserialize_from_vec(&hex::decode(signing_secret_key)?)
            .map_err(|_| Error::InvalidArgument("Signing Key".to_string()))?;
        let signing_key_pair = KeyPair::from(secret_key);

        let transaction = TransactionBuilder::new_reactivate_validator(
            &self.get_wallet_keypair(&sender_wallet)?,
            validator_address,
            &signing_key_pair,
            fee,
            self.validity_start_height(validity_start_height),
            self.get_network_id(),
        );

        Ok(transaction_to_hex_string(&transaction).into())
    }

    async fn send_reactivate_validator_transaction(
        &self,
        sender_wallet: Address,
        validator_address: Address,
        signing_secret_key: String,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<Blake2bHash, (), Self::Error> {
        // If the node is in the position of having a full state, it can check upfront if this transaction makes sense
        if let BlockchainReadProxy::Full(blockchain) = self.consensus.blockchain.read() {
            let staking_contract = blockchain
                .get_staking_contract_if_complete(None)
                .ok_or(Error::NoConsensus)?;
            let data_store = blockchain.get_staking_contract_store();
            let db_txn = blockchain.read_transaction();
            let validator =
                staking_contract.get_validator(&data_store.read(&db_txn), &validator_address);

            if let Some(validator) = validator {
                if validator.retired {
                    return Err(Error::ValidatorRetired(validator_address.clone()));
                } else if validator.is_active() {
                    return Err(Error::ValidatorAlreadyInState(
                        validator_address.clone(),
                        "active".into(),
                    ));
                }
            } else {
                return Err(Error::ValidatorNotFound(validator_address.clone()));
            }
        }

        let raw_tx = self
            .create_reactivate_validator_transaction(
                sender_wallet,
                validator_address,
                signing_secret_key,
                fee,
                validity_start_height,
            )
            .await?
            .data;
        self.send_raw_transaction(raw_tx).await
    }

    async fn create_set_signal_data_transaction(
        &self,
        sender_wallet: Address,
        validator_address: Address,
        signing_secret_key: String,
        new_signal_data: Option<String>,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<String, (), Self::Error> {
        let secret_key = PrivateKey::deserialize_from_vec(&hex::decode(signing_secret_key)?)
            .map_err(|_| Error::InvalidArgument("Signing Key".to_string()))?;
        let signing_key_pair = KeyPair::from(secret_key);

        // `None` = clear the signal data, "0x29a4b..." = set the signal data.
        let new_signal_data: Option<Blake2bHash> = new_signal_data
            .map(|string| {
                Blake2bHash::deserialize_from_vec(&hex::decode(string)?)
                    .map_err(|_| Error::InvalidArgument("Signal Data".to_string()))
            })
            .transpose()?;

        let transaction = TransactionBuilder::new_set_signal_data(
            &self.get_wallet_keypair(&sender_wallet)?,
            validator_address,
            &signing_key_pair,
            new_signal_data,
            fee,
            self.validity_start_height(validity_start_height),
            self.get_network_id(),
        );

        Ok(transaction_to_hex_string(&transaction).into())
    }

    async fn send_set_signal_data_transaction(
        &self,
        sender_wallet: Address,
        validator_address: Address,
        signing_secret_key: String,
        new_signal_data: Option<String>,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<Blake2bHash, (), Self::Error> {
        let raw_tx = self
            .create_set_signal_data_transaction(
                sender_wallet,
                validator_address,
                signing_secret_key,
                new_signal_data,
                fee,
                validity_start_height,
            )
            .await?
            .data;
        self.send_raw_transaction(raw_tx).await
    }

    async fn create_signal_version_transaction(
        &self,
        sender_wallet: Address,
        validator_address: Address,
        signing_secret_key: String,
        version: u16,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<String, (), Self::Error> {
        let secret_key = PrivateKey::deserialize_from_vec(&hex::decode(signing_secret_key)?)
            .map_err(|_| Error::InvalidArgument("Signing Key".to_string()))?;
        let signing_key_pair = KeyPair::from(secret_key);

        let transaction = TransactionBuilder::new_signal_version(
            &self.get_wallet_keypair(&sender_wallet)?,
            validator_address,
            &signing_key_pair,
            version,
            fee,
            self.validity_start_height(validity_start_height),
            self.get_network_id(),
        );

        Ok(transaction_to_hex_string(&transaction).into())
    }

    async fn send_signal_version_transaction(
        &self,
        sender_wallet: Address,
        validator_address: Address,
        signing_secret_key: String,
        version: u16,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<Blake2bHash, (), Self::Error> {
        let raw_tx = self
            .create_signal_version_transaction(
                sender_wallet,
                validator_address,
                signing_secret_key,
                version,
                fee,
                validity_start_height,
            )
            .await?
            .data;
        self.send_raw_transaction(raw_tx).await
    }

    async fn create_retire_validator_transaction(
        &self,
        sender_wallet: Address,
        validator_wallet: Address,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<String, (), Self::Error> {
        let transaction = TransactionBuilder::new_retire_validator(
            &self.get_wallet_keypair(&sender_wallet)?,
            &self.get_wallet_keypair(&validator_wallet)?,
            fee,
            self.validity_start_height(validity_start_height),
            self.get_network_id(),
        );

        Ok(transaction_to_hex_string(&transaction).into())
    }

    async fn send_retire_validator_transaction(
        &self,
        sender_wallet: Address,
        validator_wallet: Address,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<Blake2bHash, (), Self::Error> {
        // If the node is in the position of having a full state, it can check upfront if this transaction makes sense
        if let BlockchainReadProxy::Full(blockchain) = self.consensus.blockchain.read() {
            let staking_contract = blockchain
                .get_staking_contract_if_complete(None)
                .ok_or(Error::NoConsensus)?;
            let data_store = blockchain.get_staking_contract_store();
            let db_txn = blockchain.read_transaction();
            let validator =
                staking_contract.get_validator(&data_store.read(&db_txn), &validator_wallet);

            if let Some(validator) = validator {
                if validator.retired {
                    return Err(Error::ValidatorAlreadyInState(
                        validator_wallet.clone(),
                        "retired".into(),
                    ));
                }
            } else {
                return Err(Error::ValidatorNotFound(validator_wallet.clone()));
            }
        }

        let raw_tx = self
            .create_retire_validator_transaction(
                sender_wallet,
                validator_wallet,
                fee,
                validity_start_height,
            )
            .await?
            .data;
        self.send_raw_transaction(raw_tx).await
    }

    async fn create_delete_validator_transaction(
        &self,
        validator_wallet: Address,
        recipient: Address,
        fee: Coin,
        value: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<String, (), Self::Error> {
        let transaction = TransactionBuilder::new_delete_validator(
            recipient,
            &self.get_wallet_keypair(&validator_wallet)?,
            fee,
            value,
            self.validity_start_height(validity_start_height),
            self.get_network_id(),
        )?;

        Ok(transaction_to_hex_string(&transaction).into())
    }

    async fn send_delete_validator_transaction(
        &self,
        validator_wallet: Address,
        recipient: Address,
        fee: Coin,
        value: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<Blake2bHash, (), Self::Error> {
        let raw_tx = self
            .create_delete_validator_transaction(
                validator_wallet,
                recipient,
                fee,
                value,
                validity_start_height,
            )
            .await?
            .data;
        self.send_raw_transaction(raw_tx).await
    }

    async fn create_new_bridge_transaction(
        &self,
        wallet: Address,
        owner: Address,
        oracle_address: Address,
        source_chain_id: u32,
        chain_config: String,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<String, (), Self::Error> {
        use nimiq_transaction::account::bridge_contract::ChainConfig;

        // Deserialize the chain_config from hex string
        let chain_config_bytes = hex::decode(chain_config)?;
        let chain_config: ChainConfig = Deserialize::deserialize_from_vec(&chain_config_bytes)?;

        let transaction = TransactionBuilder::new_create_bridge(
            &self.get_wallet_keypair(&wallet)?,
            owner,
            oracle_address,
            source_chain_id,
            chain_config,
            value,
            fee,
            self.validity_start_height(validity_start_height),
            self.get_network_id(),
        )?;

        Ok(transaction_to_hex_string(&transaction).into())
    }

    async fn send_new_bridge_transaction(
        &self,
        wallet: Address,
        owner: Address,
        oracle_address: Address,
        source_chain_id: u32,
        chain_config: String,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<Blake2bHash, (), Self::Error> {
        let raw_tx = self
            .create_new_bridge_transaction(
                wallet,
                owner,
                oracle_address,
                source_chain_id,
                chain_config,
                value,
                fee,
                validity_start_height,
            )
            .await?
            .data;
        self.send_raw_transaction(raw_tx).await
    }

    async fn create_new_oracle_transaction(
        &self,
        wallet: Address,
        owner: Address,
        hash_count: u16,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<String, (), Self::Error> {
        let transaction = TransactionBuilder::new_create_oracle(
            &self.get_wallet_keypair(&wallet)?,
            owner,
            hash_count,
            value,
            fee,
            self.validity_start_height(validity_start_height),
            self.get_network_id(),
        )?;

        Ok(transaction_to_hex_string(&transaction).into())
    }

    async fn send_new_oracle_transaction(
        &self,
        wallet: Address,
        owner: Address,
        hash_count: u16,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<Blake2bHash, (), Self::Error> {
        let raw_tx = self
            .create_new_oracle_transaction(
                wallet,
                owner,
                hash_count,
                value,
                fee,
                validity_start_height,
            )
            .await?
            .data;
        self.send_raw_transaction(raw_tx).await
    }

    async fn create_bridge_deposit_transaction(
        &self,
        wallet: Address,
        bridge_address: Address,
        data: String,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<String, (), Self::Error> {
        let transaction = TransactionBuilder::new_bridge_deposit(
            &self.get_wallet_keypair(&wallet)?,
            bridge_address,
            hex::decode(data)?,
            value,
            fee,
            self.validity_start_height(validity_start_height),
            self.get_network_id(),
        )?;

        Ok(transaction_to_hex_string(&transaction).into())
    }

    async fn send_bridge_deposit_transaction(
        &self,
        wallet: Address,
        bridge_address: Address,
        data: String,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<Blake2bHash, (), Self::Error> {
        let raw_tx = self
            .create_bridge_deposit_transaction(
                wallet,
                bridge_address,
                data,
                value,
                fee,
                validity_start_height,
            )
            .await?
            .data;
        self.send_raw_transaction(raw_tx).await
    }

    async fn create_bridge_release_transaction(
        &self,
        signer_wallet: Address,
        bridge_address: Address,
        recipient: Address,
        burn_transaction_data: String,
        merkle_proof: String,
        oracle_state_index: u64,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<String, (), Self::Error> {
        let transaction = TransactionBuilder::new_bridge_release(
            &self.get_wallet_keypair(&signer_wallet)?,
            bridge_address,
            recipient,
            parse_burn_proof(&burn_transaction_data, &merkle_proof, oracle_state_index)?,
            value,
            fee,
            self.validity_start_height(validity_start_height),
            self.get_network_id(),
        )?;

        Ok(transaction_to_hex_string(&transaction).into())
    }

    async fn send_bridge_release_transaction(
        &self,
        signer_wallet: Address,
        bridge_address: Address,
        recipient: Address,
        burn_transaction_data: String,
        merkle_proof: String,
        oracle_state_index: u64,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<Blake2bHash, (), Self::Error> {
        let burn_proof =
            parse_burn_proof(&burn_transaction_data, &merkle_proof, oracle_state_index)?;
        self.check_bridge_release(&bridge_address, &recipient, value, &burn_proof)?;

        let raw_tx = self
            .create_bridge_release_transaction(
                signer_wallet,
                bridge_address,
                recipient,
                burn_transaction_data,
                merkle_proof,
                oracle_state_index,
                value,
                fee,
                validity_start_height,
            )
            .await?
            .data;
        self.send_raw_transaction(raw_tx).await
    }

    async fn create_update_oracle_transaction(
        &self,
        sender_wallet: Address,
        owner_wallet: Address,
        oracle_address: Address,
        hashes: Vec<AnyHash>,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<String, (), Self::Error> {
        let transaction = TransactionBuilder::new_update_oracle(
            &self.get_wallet_keypair(&sender_wallet)?,
            &self.get_wallet_keypair(&owner_wallet)?,
            oracle_address,
            hashes,
            fee,
            self.validity_start_height(validity_start_height),
            self.get_network_id(),
        );

        Ok(transaction_to_hex_string(&transaction).into())
    }

    async fn send_update_oracle_transaction(
        &self,
        sender_wallet: Address,
        owner_wallet: Address,
        oracle_address: Address,
        hashes: Vec<AnyHash>,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<Blake2bHash, (), Self::Error> {
        let raw_tx = self
            .create_update_oracle_transaction(
                sender_wallet,
                owner_wallet,
                oracle_address,
                hashes,
                fee,
                validity_start_height,
            )
            .await?
            .data;
        self.send_raw_transaction(raw_tx).await
    }

    async fn create_change_oracle_owner_transaction(
        &self,
        sender_wallet: Address,
        owner_wallet: Address,
        oracle_address: Address,
        new_owner: Address,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<String, (), Self::Error> {
        let transaction = TransactionBuilder::new_change_oracle_owner(
            &self.get_wallet_keypair(&sender_wallet)?,
            &self.get_wallet_keypair(&owner_wallet)?,
            oracle_address,
            new_owner,
            fee,
            self.validity_start_height(validity_start_height),
            self.get_network_id(),
        );

        Ok(transaction_to_hex_string(&transaction).into())
    }

    async fn send_change_oracle_owner_transaction(
        &self,
        sender_wallet: Address,
        owner_wallet: Address,
        oracle_address: Address,
        new_owner: Address,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<Blake2bHash, (), Self::Error> {
        let raw_tx = self
            .create_change_oracle_owner_transaction(
                sender_wallet,
                owner_wallet,
                oracle_address,
                new_owner,
                fee,
                validity_start_height,
            )
            .await?
            .data;
        self.send_raw_transaction(raw_tx).await
    }

    async fn create_delete_oracle_transaction(
        &self,
        owner_wallet: Address,
        oracle_address: Address,
        recipient: Address,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<String, (), Self::Error> {
        // The oracle contract only accepts a withdrawal of its full balance with no fee on top. A
        // withdrawal with a fee fails, and the contract cannot even charge the fee of the failed
        // transaction, so such a transaction must not be built.
        if !fee.is_zero() {
            return Err(Error::InvalidArgument(
                "Fee: withdrawing an oracle deposit requires a zero fee".to_string(),
            ));
        }

        let transaction = TransactionBuilder::new_delete_oracle(
            &self.get_wallet_keypair(&owner_wallet)?,
            oracle_address,
            recipient,
            value,
            fee,
            self.validity_start_height(validity_start_height),
            self.get_network_id(),
        )?;

        Ok(transaction_to_hex_string(&transaction).into())
    }

    async fn send_delete_oracle_transaction(
        &self,
        owner_wallet: Address,
        oracle_address: Address,
        recipient: Address,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<Blake2bHash, (), Self::Error> {
        // If the node is in the position of having a full state, it can check upfront if this transaction makes sense
        if let BlockchainReadProxy::Full(blockchain) = self.consensus.blockchain.read() {
            match blockchain
                .get_account_if_complete(&oracle_address)
                .ok_or(Error::NoConsensus)?
            {
                Account::Oracle(oracle) if oracle.balance != value => {
                    return Err(Error::InvalidArgument(format!(
                        "Value: must equal the oracle balance of {}",
                        oracle.balance
                    )));
                }
                Account::Oracle(_) => {}
                _ => return Err(Error::InvalidAddress(oracle_address)),
            }
        }

        let raw_tx = self
            .create_delete_oracle_transaction(
                owner_wallet,
                oracle_address,
                recipient,
                value,
                fee,
                validity_start_height,
            )
            .await?
            .data;
        self.send_raw_transaction(raw_tx).await
    }
}

#[cfg(test)]
mod tests {
    use nimiq_account::{BridgeContract, OracleContract};
    use nimiq_hash::{Blake2bHasher, HashOutput, Hasher};
    use nimiq_keys::Address;
    use nimiq_primitives::coin::Coin;
    use nimiq_transaction::account::{
        bridge_contract::{
            AddressFormat, AnyMerkleProof, ChainConfig, Endianness, OutgoingTransaction,
            ValidationOp, ValidationProgram,
        },
        htlc_contract::{AnyHash, AnyHash32},
    };
    use nimiq_utils::merkle::MerklePath;

    use super::verify_bridge_release;

    const CHAIN_ID: u32 = 1;
    const AMOUNT: u64 = 500;

    fn target() -> Address {
        Address::from([0xAAu8; 20])
    }

    fn blake2b(data: &[u8]) -> AnyHash {
        AnyHash::Blake2b(AnyHash32::from(
            Blake2bHasher::default().digest(data).as_bytes(),
        ))
    }

    /// Reads `[0..20]` target address, `[20..28]` amount, `[28..36]` nonce,
    /// `[36..40]` burn block height and `[40..44]` target chain ID.
    fn bridge(balance: u64) -> BridgeContract {
        let load = |offset, op, name: &str| {
            [
                ValidationOp::PushConst(offset),
                op,
                ValidationOp::Store(name.to_string()),
            ]
        };
        let operations = [
            load(0, ValidationOp::LoadAddress, "target_address"),
            load(
                20,
                ValidationOp::LoadU64(Endianness::LittleEndian),
                "amount",
            ),
            load(
                28,
                ValidationOp::LoadU64(Endianness::LittleEndian),
                "target_nonce",
            ),
            load(
                36,
                ValidationOp::LoadU32(Endianness::LittleEndian),
                "burn_block_height",
            ),
            load(
                40,
                ValidationOp::LoadU32(Endianness::LittleEndian),
                "target_chain_id",
            ),
        ]
        .concat();

        BridgeContract {
            owner: Address::from([0x01u8; 20]),
            oracle_address: Address::from([0x0Eu8; 20]),
            balance: Coin::from_u64_unchecked(balance),
            source_chain_id: CHAIN_ID,
            chain_config: ChainConfig {
                chain_id: CHAIN_ID,
                hash_function: AnyHash::Blake2b(AnyHash32::default()),
                address_format: AddressFormat::Nimiq,
                endianness: Endianness::LittleEndian,
                block_time: std::time::Duration::from_secs(60),
                validation_program: ValidationProgram::new(operations),
                max_proof_depth: 64,
            },
            transaction_count: 0,
        }
    }

    fn burn_data(nonce: u64) -> Vec<u8> {
        let mut data = target().as_bytes().to_vec();
        data.extend_from_slice(&AMOUNT.to_le_bytes());
        data.extend_from_slice(&nonce.to_le_bytes());
        data.extend_from_slice(&42u32.to_le_bytes());
        data.extend_from_slice(&CHAIN_ID.to_le_bytes());
        data
    }

    fn burn_proof(burn_data: Vec<u8>, oracle_state_index: u64) -> OutgoingTransaction {
        OutgoingTransaction::new(
            burn_data,
            AnyMerkleProof::Blake2bPath(MerklePath::empty()),
            oracle_state_index,
        )
        .unwrap()
    }

    /// An oracle whose states commit to single-leaf trees holding the given burn transactions,
    /// chained the same way the oracle contract chains its updates.
    fn oracle(burns: &[Vec<u8>]) -> OracleContract {
        let hash_count = 4;
        let mut hashes = vec![blake2b(&[]).zero_of_same_type(); hash_count];
        let mut state = hashes[0].clone();
        for (index, burn) in burns.iter().enumerate() {
            state = state.digest(&blake2b(burn));
            hashes[index] = state.clone();
        }
        OracleContract {
            owner: Address::from([0x01u8; 20]),
            balance: Coin::from_u64_unchecked(1_000),
            hash_count: hash_count as u16,
            hashes,
            latest_index: Some(burns.len() as u64 - 1),
        }
    }

    fn verify(
        bridge: &BridgeContract,
        oracle: Option<&OracleContract>,
        nonce: u64,
        value: u64,
        burn_proof: &OutgoingTransaction,
    ) -> Result<(), String> {
        verify_bridge_release(
            bridge,
            oracle,
            nonce,
            &target(),
            Coin::from_u64_unchecked(value),
            burn_proof,
        )
    }

    #[test]
    fn accepts_releases_proven_against_the_first_and_a_chained_state() {
        let oracle = oracle(&[burn_data(1), burn_data(2)]);

        let first = burn_proof(burn_data(1), 0);
        assert_eq!(
            verify(&bridge(10_000), Some(&oracle), 0, AMOUNT, &first),
            Ok(())
        );

        let second = burn_proof(burn_data(2), 1);
        assert_eq!(
            verify(&bridge(10_000), Some(&oracle), 1, AMOUNT, &second),
            Ok(())
        );
    }

    #[test]
    fn rejects_releases_that_consensus_would_reject() {
        let oracle = oracle(&[burn_data(1)]);
        let proof = burn_proof(burn_data(1), 0);
        let rejects = |result: Result<(), String>, reason: &str| {
            let error = result.expect_err("the release must be rejected");
            assert!(
                error.contains(reason),
                "{error:?} does not mention {reason:?}"
            );
        };

        rejects(
            verify(&bridge(10_000), Some(&oracle), 0, AMOUNT + 1, &proof),
            "burned amount",
        );
        rejects(
            verify_bridge_release(
                &bridge(10_000),
                Some(&oracle),
                0,
                &Address::from([0xBBu8; 20]),
                Coin::from_u64_unchecked(AMOUNT),
                &proof,
            ),
            "pays",
        );
        rejects(
            verify(&bridge(10_000), Some(&oracle), 1, AMOUNT, &proof),
            "nonce",
        );
        rejects(
            verify(&bridge(10_000), None, 0, AMOUNT, &proof),
            "does not exist",
        );
        rejects(
            verify(
                &bridge(10_000),
                Some(&oracle),
                0,
                AMOUNT,
                &burn_proof(burn_data(1), 1),
            ),
            "not available",
        );
        rejects(
            verify(
                &bridge(10_000),
                Some(&oracle),
                1,
                AMOUNT,
                &burn_proof(burn_data(2), 0),
            ),
            "does not match",
        );
        rejects(
            verify(&bridge(AMOUNT - 1), Some(&oracle), 0, AMOUNT, &proof),
            "only holds",
        );
    }
}
