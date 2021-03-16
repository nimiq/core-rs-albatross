use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::RwLock;

use beserial::{Deserialize, Serialize};
use nimiq_bls::{KeyPair as BlsKeyPair, SecretKey as BlsSecretKey};
use nimiq_consensus_albatross::ConsensusProxy;
use nimiq_hash::{Blake2bHash, Hash};
use nimiq_keys::{Address, KeyPair};
use nimiq_mempool::ReturnCode;
use nimiq_network_libp2p::Network;
use nimiq_primitives::{account::ValidatorId, coin::Coin, networks::NetworkId};
use nimiq_transaction::Transaction;
use nimiq_transaction_builder::TransactionBuilder;

use nimiq_rpc_interface::{consensus::ConsensusInterface, types::ValidityStartHeight};

use crate::{error::Error, wallets::UnlockedWallets};
use nimiq_blockchain_albatross::AbstractBlockchain;

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

    async fn push_transaction(&self, tx: Transaction) -> Result<Blake2bHash, Error> {
        let txid = tx.hash::<Blake2bHash>();
        match self.consensus.send_transaction(tx).await {
            Ok(ReturnCode::Accepted) => Ok(txid),
            Ok(return_code) => Err(Error::TransactionRejected(return_code)),
            Err(e) => Err(Error::NetworkError(e)),
        }
    }

    fn get_wallet_keypair(&self, address: &Address) -> Result<KeyPair, Error> {
        Ok(self
            .unlocked_wallets
            .as_ref()
            .ok_or_else(|| Error::UnlockedWalletNotFound(address.clone()))?
            .read()
            .get(address)
            .ok_or_else(|| Error::UnlockedWalletNotFound(address.clone()))?
            .key_pair
            .clone()) // TODO: Avoid cloning
    }

    fn network_id(&self) -> NetworkId {
        self.consensus.blockchain.network_id
    }

    fn validity_start_height(&self, validity_start_height: ValidityStartHeight) -> u32 {
        validity_start_height.block_number(self.consensus.blockchain.block_number())
    }
}

fn transaction_to_hex_string(transaction: &Transaction) -> String {
    hex::encode(&transaction.serialize_to_vec())
}

#[nimiq_jsonrpc_derive::service(rename_all = "camelCase")]
#[async_trait]
impl ConsensusInterface for ConsensusDispatcher {
    type Error = Error;

    async fn is_established(&mut self) -> Result<bool, Self::Error> {
        Ok(self.consensus.is_established())
    }

    async fn send_raw_transaction(&mut self, raw_tx: String) -> Result<Blake2bHash, Error> {
        let tx = Deserialize::deserialize_from_vec(&hex::decode(&raw_tx)?)?;
        self.push_transaction(tx).await
    }

    async fn create_basic_transaction(
        &mut self,
        wallet: Address,
        recipient: Address,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<String, Error> {
        let transaction = TransactionBuilder::new_simple(
            &self.get_wallet_keypair(&wallet)?,
            recipient,
            value,
            fee,
            self.validity_start_height(validity_start_height),
            self.network_id(),
        );

        Ok(transaction_to_hex_string(&transaction))
    }

    async fn send_basic_transaction(
        &mut self,
        wallet: Address,
        recipient: Address,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<Blake2bHash, Error> {
        let raw_tx = self
            .create_basic_transaction(wallet, recipient, value, fee, validity_start_height)
            .await
            .unwrap();
        self.send_raw_transaction(raw_tx).await
    }

    async fn create_stake_transaction(
        &mut self,
        wallet: Address,
        validator_id: ValidatorId,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<String, Error> {
        let transaction = TransactionBuilder::new_stake(
            None,
            &self.get_wallet_keypair(&wallet)?,
            &validator_id,
            value,
            fee,
            self.validity_start_height(validity_start_height),
            self.network_id(),
        );

        Ok(transaction_to_hex_string(&transaction))
    }

    async fn send_stake_transaction(
        &mut self,
        wallet: Address,
        validator_id: ValidatorId,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<Blake2bHash, Error> {
        let raw_tx = self
            .create_stake_transaction(wallet, validator_id, value, fee, validity_start_height)
            .await
            .unwrap();
        self.send_raw_transaction(raw_tx).await
    }

    async fn create_rededicate_transaction(
        &mut self,
        wallet: Address,
        from_validator_id: ValidatorId,
        to_validator_id: ValidatorId,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<String, Error> {
        let transaction = TransactionBuilder::new_rededicate_stake(
            None,
            &self.get_wallet_keypair(&wallet)?,
            &from_validator_id,
            &to_validator_id,
            value,
            fee,
            self.validity_start_height(validity_start_height),
            self.network_id(),
        );

        Ok(transaction_to_hex_string(&transaction))
    }

    async fn send_rededicate_transaction(
        &mut self,
        wallet: Address,
        from_validator_id: ValidatorId,
        to_validator_id: ValidatorId,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<Blake2bHash, Error> {
        let raw_tx = self
            .create_rededicate_transaction(
                wallet,
                from_validator_id,
                to_validator_id,
                value,
                fee,
                validity_start_height,
            )
            .await
            .unwrap();
        self.send_raw_transaction(raw_tx).await
    }

    async fn create_retire_transaction(
        &mut self,
        wallet: Address,
        validator_id: ValidatorId,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<String, Error> {
        let transaction = TransactionBuilder::new_retire(
            None,
            &self.get_wallet_keypair(&wallet)?,
            &validator_id,
            value,
            fee,
            self.validity_start_height(validity_start_height),
            self.network_id(),
        );

        Ok(transaction_to_hex_string(&transaction))
    }

    async fn send_retire_transaction(
        &mut self,
        wallet: Address,
        validator_id: ValidatorId,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<Blake2bHash, Error> {
        let raw_tx = self
            .create_retire_transaction(wallet, validator_id, value, fee, validity_start_height)
            .await
            .unwrap();
        self.send_raw_transaction(raw_tx).await
    }

    async fn create_reactivate_transaction(
        &mut self,
        wallet: Address,
        validator_id: ValidatorId,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<String, Error> {
        let transaction = TransactionBuilder::new_reactivate(
            None,
            &self.get_wallet_keypair(&wallet)?,
            &validator_id,
            value,
            fee,
            self.validity_start_height(validity_start_height),
            self.network_id(),
        );

        Ok(transaction_to_hex_string(&transaction))
    }

    async fn send_reactivate_transaction(
        &mut self,
        wallet: Address,
        validator_id: ValidatorId,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<Blake2bHash, Error> {
        let raw_tx = self
            .create_reactivate_transaction(wallet, validator_id, value, fee, validity_start_height)
            .await
            .unwrap();
        self.send_raw_transaction(raw_tx).await
    }

    async fn create_unstake_transaction(
        &mut self,
        wallet: Address,
        recipient: Address,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<String, Error> {
        let transaction = TransactionBuilder::new_unstake(
            None,
            &self.get_wallet_keypair(&wallet)?,
            recipient,
            value,
            fee,
            self.validity_start_height(validity_start_height),
            self.network_id(),
        );

        Ok(transaction_to_hex_string(&transaction))
    }

    async fn send_unstake_transaction(
        &mut self,
        wallet: Address,
        recipient: Address,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<Blake2bHash, Error> {
        let raw_tx = self
            .create_unstake_transaction(wallet, recipient, value, fee, validity_start_height)
            .await
            .unwrap();
        self.send_raw_transaction(raw_tx).await
    }

    async fn create_new_validator_transaction(
        &mut self,
        wallet: Address,
        reward_address: Address,
        validator_secret_key: String,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<String, Error> {
        let secret_key =
            BlsSecretKey::deserialize_from_vec(&hex::decode(validator_secret_key).unwrap())
                .unwrap();
        let validator_keypair = BlsKeyPair::from(secret_key);

        let transaction = TransactionBuilder::new_create_validator(
            None,
            &self.get_wallet_keypair(&wallet)?,
            reward_address,
            &validator_keypair,
            value,
            fee,
            self.validity_start_height(validity_start_height),
            self.network_id(),
        );

        Ok(transaction_to_hex_string(&transaction))
    }

    async fn send_new_validator_transaction(
        &mut self,
        wallet: Address,
        reward_address: Address,
        validator_secret_key: String,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<Blake2bHash, Error> {
        let raw_tx = self
            .create_new_validator_transaction(
                wallet,
                reward_address,
                validator_secret_key,
                value,
                fee,
                validity_start_height,
            )
            .await
            .unwrap();
        self.send_raw_transaction(raw_tx).await
    }

    async fn create_update_validator_transaction(
        &mut self,
        wallet: Address,
        validator_id: ValidatorId,
        new_reward_address: Option<Address>,
        old_validator_secret_key: String,
        new_validator_secret_key: Option<String>,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<String, Error> {
        let old_secret_key =
            BlsSecretKey::deserialize_from_vec(&hex::decode(old_validator_secret_key).unwrap())
                .unwrap();
        let old_validator_keypair = BlsKeyPair::from(old_secret_key);

        let new_validator_keypair = match new_validator_secret_key {
            Some(key) => {
                let new_secret_key =
                    BlsSecretKey::deserialize_from_vec(&hex::decode(key).unwrap()).unwrap();
                Some(BlsKeyPair::from(new_secret_key))
            }
            _ => None,
        };

        let transaction = TransactionBuilder::new_update_validator(
            None,
            &self.get_wallet_keypair(&wallet)?,
            &validator_id,
            new_reward_address,
            &old_validator_keypair,
            new_validator_keypair.as_ref(),
            fee,
            self.validity_start_height(validity_start_height),
            self.network_id(),
        );

        Ok(transaction_to_hex_string(&transaction))
    }

    async fn send_update_validator_transaction(
        &mut self,
        wallet: Address,
        validator_id: ValidatorId,
        new_reward_address: Option<Address>,
        old_validator_secret_key: String,
        new_validator_secret_key: Option<String>,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<Blake2bHash, Error> {
        let raw_tx = self
            .create_update_validator_transaction(
                wallet,
                validator_id,
                new_reward_address,
                old_validator_secret_key,
                new_validator_secret_key,
                fee,
                validity_start_height,
            )
            .await
            .unwrap();
        self.send_raw_transaction(raw_tx).await
    }

    async fn create_retire_validator_transaction(
        &mut self,
        wallet: Address,
        validator_id: ValidatorId,
        validator_secret_key: String,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<String, Error> {
        let secret_key =
            BlsSecretKey::deserialize_from_vec(&hex::decode(validator_secret_key).unwrap())
                .unwrap();
        let validator_keypair = BlsKeyPair::from(secret_key);

        let transaction = TransactionBuilder::new_retire_validator(
            None,
            &self.get_wallet_keypair(&wallet)?,
            &validator_id,
            &validator_keypair,
            fee,
            self.validity_start_height(validity_start_height),
            self.network_id(),
        );

        Ok(transaction_to_hex_string(&transaction))
    }

    async fn send_retire_validator_transaction(
        &mut self,
        wallet: Address,
        validator_id: ValidatorId,
        validator_secret_key: String,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<Blake2bHash, Error> {
        let raw_tx = self
            .create_retire_validator_transaction(
                wallet,
                validator_id,
                validator_secret_key,
                fee,
                validity_start_height,
            )
            .await
            .unwrap();
        self.send_raw_transaction(raw_tx).await
    }

    async fn create_reactivate_validator_transaction(
        &mut self,
        wallet: Address,
        validator_id: ValidatorId,
        validator_secret_key: String,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<String, Error> {
        let secret_key =
            BlsSecretKey::deserialize_from_vec(&hex::decode(validator_secret_key).unwrap())
                .unwrap();
        let validator_keypair = BlsKeyPair::from(secret_key);

        let transaction = TransactionBuilder::new_reactivate_validator(
            None,
            &self.get_wallet_keypair(&wallet)?,
            &validator_id,
            &validator_keypair,
            fee,
            self.validity_start_height(validity_start_height),
            self.network_id(),
        );

        Ok(transaction_to_hex_string(&transaction))
    }

    async fn send_reactivate_validator_transaction(
        &mut self,
        wallet: Address,
        validator_id: ValidatorId,
        validator_secret_key: String,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<Blake2bHash, Error> {
        let raw_tx = self
            .create_reactivate_validator_transaction(
                wallet,
                validator_id,
                validator_secret_key,
                fee,
                validity_start_height,
            )
            .await
            .unwrap();
        self.send_raw_transaction(raw_tx).await
    }

    async fn create_drop_validator_transaction(
        &mut self,
        validator_id: ValidatorId,
        recipient: Address,
        validator_secret_key: String,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<String, Error> {
        let secret_key =
            BlsSecretKey::deserialize_from_vec(&hex::decode(validator_secret_key).unwrap())
                .unwrap();
        let validator_keypair = BlsKeyPair::from(secret_key);

        let transaction = TransactionBuilder::new_drop_validator(
            None,
            &validator_id,
            recipient,
            &validator_keypair,
            value,
            fee,
            self.validity_start_height(validity_start_height),
            self.network_id(),
        );

        Ok(transaction_to_hex_string(&transaction))
    }

    async fn send_drop_validator_transaction(
        &mut self,
        validator_id: ValidatorId,
        recipient: Address,
        validator_secret_key: String,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<Blake2bHash, Error> {
        let raw_tx = self
            .create_drop_validator_transaction(
                validator_id,
                recipient,
                validator_secret_key,
                value,
                fee,
                validity_start_height,
            )
            .await
            .unwrap();
        self.send_raw_transaction(raw_tx).await
    }

    async fn create_unpark_validator_transaction(
        &mut self,
        wallet: Address,
        validator_id: ValidatorId,
        validator_secret_key: String,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<String, Error> {
        let secret_key =
            BlsSecretKey::deserialize_from_vec(&hex::decode(validator_secret_key).unwrap())
                .unwrap();
        let validator_keypair = BlsKeyPair::from(secret_key);

        let transaction = TransactionBuilder::new_unpark_validator(
            None,
            &self.get_wallet_keypair(&wallet)?,
            &validator_id,
            &validator_keypair,
            fee,
            self.validity_start_height(validity_start_height),
            self.network_id(),
        );

        Ok(transaction_to_hex_string(&transaction))
    }

    async fn send_unpark_validator_transaction(
        &mut self,
        wallet: Address,
        validator_id: ValidatorId,
        validator_secret_key: String,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<Blake2bHash, Error> {
        let raw_tx = self
            .create_unpark_validator_transaction(
                wallet,
                validator_id,
                validator_secret_key,
                fee,
                validity_start_height,
            )
            .await
            .unwrap();
        self.send_raw_transaction(raw_tx).await
    }
}
