use std::sync::Arc;

use async_trait::async_trait;
use nimiq_blockchain_interface::AbstractBlockchain;
use nimiq_blockchain_proxy::BlockchainReadProxy;
use nimiq_bls::{KeyPair as BlsKeyPair, SecretKey as BlsSecretKey};
use nimiq_consensus::ConsensusProxy;
use nimiq_hash::{Blake2bHash, Hash};
use nimiq_keys::{Address, KeyPair, PrivateKey, PublicKey};
use nimiq_network_libp2p::Network;
use nimiq_primitives::{coin::Coin, networks::NetworkId};
use nimiq_rpc_interface::{
    consensus::ConsensusInterface,
    types::{RPCResult, Transaction as RPCTransaction, ValidityStartHeight},
};
use nimiq_serde::{Deserialize, Serialize};
use nimiq_transaction::{
    account::htlc_contract::{AnyHash, PreImage},
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
}

fn transaction_to_hex_string(transaction: &Transaction) -> String {
    hex::encode(transaction.serialize_to_vec())
}

#[nimiq_jsonrpc_derive::service(rename_all = "camelCase")]
#[async_trait]
impl ConsensusInterface for ConsensusDispatcher {
    type Error = Error;

    /// Returns a boolean specifying if we have established consensus with the network.
    async fn is_consensus_established(&mut self) -> RPCResult<bool, (), Self::Error> {
        Ok(self.consensus.is_established().into())
    }

    /// Given a serialized transaction, it will return the corresponding transaction struct.
    async fn get_raw_transaction_info(
        &mut self,
        raw_tx: String,
    ) -> RPCResult<RPCTransaction, (), Self::Error> {
        let transaction: Transaction = Deserialize::deserialize_from_vec(&hex::decode(raw_tx)?)?;
        Ok(RPCTransaction::from_transaction(transaction).into())
    }

    /// Sends the given serialized transaction to the network.
    async fn send_raw_transaction(
        &mut self,
        raw_tx: String,
    ) -> RPCResult<Blake2bHash, (), Self::Error> {
        let tx: Transaction = Deserialize::deserialize_from_vec(&hex::decode(&raw_tx)?)?;
        let txid = tx.hash::<Blake2bHash>();

        match self.consensus.send_transaction(tx).await {
            Ok(_) => Ok(txid.into()),
            Err(e) => Err(Error::NetworkError(e)),
        }
    }

    /// Returns a serialized basic transaction.
    async fn create_basic_transaction(
        &mut self,
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

    /// Sends a basic transaction to the network.
    async fn send_basic_transaction(
        &mut self,
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

    /// Returns a serialized basic transaction with an arbitrary data field.
    async fn create_basic_transaction_with_data(
        &mut self,
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
            hex::decode(data).unwrap(),
            value,
            fee,
            self.validity_start_height(validity_start_height),
            self.get_network_id(),
        )?;

        Ok(transaction_to_hex_string(&transaction).into())
    }

    /// Sends a basic transaction, with an arbitrary data field, to the network.
    async fn send_basic_transaction_with_data(
        &mut self,
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

    /// Returns a serialized transaction creating a new vesting contract.
    async fn create_new_vesting_transaction(
        &mut self,
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

    /// Sends a transaction creating a new vesting contract to the network.
    async fn send_new_vesting_transaction(
        &mut self,
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

    /// Returns a serialized transaction redeeming a vesting contract.
    async fn create_redeem_vesting_transaction(
        &mut self,
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

    /// Sends a transaction redeeming a vesting contract to the network.
    async fn send_redeem_vesting_transaction(
        &mut self,
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

    /// Returns a serialized transaction creating a new HTLC contract.
    async fn create_new_htlc_transaction(
        &mut self,
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

    /// Sends a transaction creating a new HTLC contract to the network.
    async fn send_new_htlc_transaction(
        &mut self,
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

    /// Returns a serialized transaction redeeming a HTLC contract using the `RegularTransfer`
    /// method.
    async fn create_redeem_regular_htlc_transaction(
        &mut self,
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

    /// Sends a transaction redeeming a HTLC contract, using the `RegularTransfer` method, to the
    /// network.
    async fn send_redeem_regular_htlc_transaction(
        &mut self,
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

    /// Returns a serialized transaction redeeming a HTLC contract using the `TimeoutResolve`
    /// method.
    async fn create_redeem_timeout_htlc_transaction(
        &mut self,
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

    /// Sends a transaction redeeming a HTLC contract, using the `TimeoutResolve` method, to the
    /// network.
    async fn send_redeem_timeout_htlc_transaction(
        &mut self,
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

    /// Returns a serialized transaction redeeming a HTLC contract using the `EarlyResolve`
    /// method.
    async fn create_redeem_early_htlc_transaction(
        &mut self,
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

    /// Sends a transaction redeeming a HTLC contract, using the `EarlyResolve` method, to the
    /// network.
    async fn send_redeem_early_htlc_transaction(
        &mut self,
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

    /// Returns a serialized signature that can be used to redeem funds from a HTLC contract using
    /// the `EarlyResolve` method.
    async fn sign_redeem_early_htlc_transaction(
        &mut self,
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

    /// Returns a serialized `new_staker` transaction. You need to provide the address of a basic
    /// account (the sender wallet) to pay the transaction fee.
    async fn create_new_staker_transaction(
        &mut self,
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

    /// Sends a `new_staker` transaction to the network. You need to provide the address of a basic
    /// account (the sender wallet) to pay the transaction fee.
    async fn send_new_staker_transaction(
        &mut self,
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

    /// Returns a serialized `stake` transaction. The funds to be staked and the transaction fee will
    /// be paid from the `sender_wallet`.
    async fn create_stake_transaction(
        &mut self,
        sender_wallet: Address,
        staker_address: Address,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<String, (), Self::Error> {
        let transaction = TransactionBuilder::new_stake(
            &self.get_wallet_keypair(&sender_wallet)?,
            staker_address,
            value,
            fee,
            self.validity_start_height(validity_start_height),
            self.get_network_id(),
        )?;

        Ok(transaction_to_hex_string(&transaction).into())
    }

    /// Sends a `stake` transaction to the network. The funds to be staked and the transaction fee will
    /// be paid from the `sender_wallet`.
    async fn send_stake_transaction(
        &mut self,
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

    /// Returns a serialized `update_staker` transaction. You can pay the transaction fee from a basic
    /// account (by providing the sender wallet) or from the staker account's balance (by not
    /// providing a sender wallet).
    async fn create_update_staker_transaction(
        &mut self,
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

    /// Sends a `update_staker` transaction to the network. You can pay the transaction fee from a basic
    /// account (by providing the sender wallet) or from the staker account's balance (by not
    /// providing a sender wallet).
    async fn send_update_staker_transaction(
        &mut self,
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

    /// Returns a serialized `set_active_stake` transaction. You can pay the transaction fee from a basic
    /// account (by providing the sender wallet) or from the staker account's balance (by not
    /// providing a sender wallet).
    async fn create_set_active_stake_transaction(
        &mut self,
        sender_wallet: Option<Address>,
        staker_wallet: Address,
        value: Coin,
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
            value,
            fee,
            self.validity_start_height(validity_start_height),
            self.get_network_id(),
        )?;

        Ok(transaction_to_hex_string(&transaction).into())
    }

    /// Sends a `set_active_stake` transaction to the network. You can pay the transaction fee from a basic
    /// account (by providing the sender wallet) or from the staker account's balance (by not
    /// providing a sender wallet).
    async fn send_set_active_stake_transaction(
        &mut self,
        sender_wallet: Option<Address>,
        staker_wallet: Address,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<Blake2bHash, (), Self::Error> {
        let raw_tx = self
            .create_set_active_stake_transaction(
                sender_wallet,
                staker_wallet,
                value,
                fee,
                validity_start_height,
            )
            .await?
            .data;
        self.send_raw_transaction(raw_tx).await
    }

    /// Returns a serialized `unstake` transaction. The transaction fee will be paid from the funds
    /// being unstaked.
    async fn create_unstake_transaction(
        &mut self,
        staker_wallet: Address,
        recipient: Address,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<String, (), Self::Error> {
        let transaction = TransactionBuilder::new_unstake(
            &self.get_wallet_keypair(&staker_wallet)?,
            recipient,
            value,
            fee,
            self.validity_start_height(validity_start_height),
            self.get_network_id(),
        )?;

        Ok(transaction_to_hex_string(&transaction).into())
    }

    /// Sends a `unstake` transaction to the network. The transaction fee will be paid from the funds
    /// being unstaked.
    async fn send_unstake_transaction(
        &mut self,
        staker_wallet: Address,
        recipient: Address,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<Blake2bHash, (), Self::Error> {
        let raw_tx = self
            .create_unstake_transaction(staker_wallet, recipient, value, fee, validity_start_height)
            .await?
            .data;
        self.send_raw_transaction(raw_tx).await
    }

    /// Returns a serialized `new_validator` transaction. You need to provide the address of a basic
    /// account (the sender wallet) to pay the transaction fee and the validator deposit.
    /// Since JSON doesn't have a primitive for Option (it just has the null primitive), we can't
    /// have a double Option. So we use the following work-around for the signal data:
    /// "" = Set the signal data field to None.
    /// "0x29a4b..." = Set the signal data field to Some(0x29a4b...).
    async fn create_new_validator_transaction(
        &mut self,
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
        let signing_key = PublicKey::from(&signing_secret_key);

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

    /// Sends a `new_validator` transaction to the network. You need to provide the address of a basic
    /// account (the sender wallet) to pay the transaction fee and the validator deposit.
    /// Since JSON doesn't have a primitive for Option (it just has the null primitive), we can't
    /// have a double Option. So we use the following work-around for the signal data:
    /// "" = Set the signal data field to None.
    /// "0x29a4b..." = Set the signal data field to Some(0x29a4b...).
    async fn send_new_validator_transaction(
        &mut self,
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

    /// Returns a serialized `update_validator` transaction. You need to provide the address of a basic
    /// account (the sender wallet) to pay the transaction fee.
    /// Since JSON doesn't have a primitive for Option (it just has the null primitive), we can't
    /// have a double Option. So we use the following work-around for the signal data:
    /// null = No change in the signal data field.
    /// "" = Change the signal data field to None.
    /// "0x29a4b..." = Change the signal data field to Some(0x29a4b...).
    async fn create_update_validator_transaction(
        &mut self,
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
                Some(PublicKey::from(&secret_key))
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
        )?;

        Ok(transaction_to_hex_string(&transaction).into())
    }

    /// Sends a `update_validator` transaction to the network. You need to provide the address of a basic
    /// account (the sender wallet) to pay the transaction fee.
    ///  Since JSON doesn't have a primitive for Option (it just has the null primitive), we can't
    /// have a double Option. So we use the following work-around for the signal data:
    ///  null = No change in the signal data field.
    ///  "" = Change the signal data field to None.
    ///  "0x29a4b..." = Change the signal data field to Some(0x29a4b...).
    async fn send_update_validator_transaction(
        &mut self,
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

    /// Returns a serialized `deactivate_validator` transaction. You need to provide the address of a basic
    /// account (the sender wallet) to pay the transaction fee.
    async fn create_deactivate_validator_transaction(
        &mut self,
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
        )?;

        Ok(transaction_to_hex_string(&transaction).into())
    }

    /// Sends a `deactivate_validator` transaction to the network. You need to provide the address of a basic
    /// account (the sender wallet) to pay the transaction fee.
    async fn send_deactivate_validator_transaction(
        &mut self,
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

    /// Returns a serialized `reactivate_validator` transaction. You need to provide the address of a basic
    /// account (the sender wallet) to pay the transaction fee.
    async fn create_reactivate_validator_transaction(
        &mut self,
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
        )?;

        Ok(transaction_to_hex_string(&transaction).into())
    }

    /// Sends a `reactivate_validator` transaction to the network. You need to provide the address of a basic
    /// account (the sender wallet) to pay the transaction fee.
    async fn send_reactivate_validator_transaction(
        &mut self,
        sender_wallet: Address,
        validator_address: Address,
        signing_secret_key: String,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<Blake2bHash, (), Self::Error> {
        // If the node is in the position of having a full state, it can check upfront if this transaction makes sense
        if let BlockchainReadProxy::Full(blockchain) = self.consensus.blockchain.read() {
            let staking_contract = blockchain.get_staking_contract();
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

    /// Returns a serialized `retire_validator` transaction. You need to provide the address of a basic
    /// account (the sender wallet) to pay the transaction fee.
    async fn create_retire_validator_transaction(
        &mut self,
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
        )?;

        Ok(transaction_to_hex_string(&transaction).into())
    }

    /// Sends a `retire_validator` transaction to the network. You need to provide the address of a basic
    /// account (the sender wallet) to pay the transaction fee.
    async fn send_retire_validator_transaction(
        &mut self,
        sender_wallet: Address,
        validator_wallet: Address,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<Blake2bHash, (), Self::Error> {
        // If the node is in the position of having a full state, it can check upfront if this transaction makes sense
        if let BlockchainReadProxy::Full(blockchain) = self.consensus.blockchain.read() {
            let staking_contract = blockchain.get_staking_contract();
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

    /// Returns a serialized `delete_validator` transaction. The transaction fee will be paid from the
    /// validator deposit that is being returned.
    /// Note in order for this transaction to be accepted fee + value should be equal to the validator deposit, which is not a fixed value:
    /// Failed delete validator transactions can diminish the validator deposit
    async fn create_delete_validator_transaction(
        &mut self,
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

    /// Sends a `delete_validator` transaction to the network. The transaction fee will be paid from the
    /// validator deposit that is being returned.
    async fn send_delete_validator_transaction(
        &mut self,
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
}
