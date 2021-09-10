use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::RwLock;

use beserial::{Deserialize, Serialize};
use nimiq_bls::{KeyPair as BlsKeyPair, SecretKey as BlsSecretKey};
use nimiq_consensus::ConsensusProxy;
use nimiq_hash::{Blake2bHash, Hash};
use nimiq_keys::{Address, KeyPair, PrivateKey};
use nimiq_mempool::ReturnCode;
use nimiq_network_libp2p::Network;
use nimiq_primitives::{coin::Coin, networks::NetworkId};
use nimiq_transaction::Transaction;
use nimiq_transaction_builder::TransactionBuilder;

use nimiq_rpc_interface::{consensus::ConsensusInterface, types::ValidityStartHeight};

use crate::{error::Error, wallets::UnlockedWallets};
use nimiq_blockchain::AbstractBlockchain;

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
        self.consensus.blockchain.read().network_id
    }

    fn validity_start_height(&self, validity_start_height: ValidityStartHeight) -> u32 {
        validity_start_height.block_number(self.consensus.blockchain.read().block_number())
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
            .await?;
        self.send_raw_transaction(raw_tx).await
    }

    async fn create_new_staker_transaction(
        &mut self,
        wallet: Address,
        delegation: Option<Address>,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<String, Self::Error> {
        let transaction = TransactionBuilder::new_create_staker(
            &self.get_wallet_keypair(&wallet)?,
            &self.get_wallet_keypair(&wallet)?,
            delegation,
            value,
            fee,
            self.validity_start_height(validity_start_height),
            self.network_id(),
        );

        Ok(transaction_to_hex_string(&transaction))
    }

    async fn send_new_staker_transaction(
        &mut self,
        wallet: Address,
        delegation: Option<Address>,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<Blake2bHash, Self::Error> {
        let raw_tx = self
            .create_new_staker_transaction(wallet, delegation, value, fee, validity_start_height)
            .await?;
        self.send_raw_transaction(raw_tx).await
    }

    async fn create_stake_transaction(
        &mut self,
        wallet: Address,
        staker_address: Address,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<String, Error> {
        let transaction = TransactionBuilder::new_stake(
            &self.get_wallet_keypair(&wallet)?,
            staker_address,
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
        staker_address: Address,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<Blake2bHash, Error> {
        let raw_tx = self
            .create_stake_transaction(wallet, staker_address, value, fee, validity_start_height)
            .await?;
        self.send_raw_transaction(raw_tx).await
    }

    async fn create_update_transaction(
        &mut self,
        wallet: Address,
        new_delegation: Option<Address>,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<String, Error> {
        let transaction = TransactionBuilder::new_update_staker(
            &self.get_wallet_keypair(&wallet)?,
            new_delegation,
            true,
            fee,
            self.validity_start_height(validity_start_height),
            self.network_id(),
        );

        Ok(transaction_to_hex_string(&transaction))
    }

    async fn send_update_transaction(
        &mut self,
        wallet: Address,
        new_delegation: Option<Address>,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<Blake2bHash, Error> {
        let raw_tx = self
            .create_update_transaction(wallet, new_delegation, fee, validity_start_height)
            .await?;
        self.send_raw_transaction(raw_tx).await
    }

    async fn create_retire_transaction(
        &mut self,
        wallet: Address,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<String, Error> {
        let transaction = TransactionBuilder::new_retire_staker(
            &self.get_wallet_keypair(&wallet)?,
            true,
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
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<Blake2bHash, Error> {
        let raw_tx = self
            .create_retire_transaction(wallet, value, fee, validity_start_height)
            .await?;
        self.send_raw_transaction(raw_tx).await
    }

    async fn create_reactivate_transaction(
        &mut self,
        wallet: Address,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<String, Error> {
        let transaction = TransactionBuilder::new_reactivate_staker(
            &self.get_wallet_keypair(&wallet)?,
            false,
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
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<Blake2bHash, Error> {
        let raw_tx = self
            .create_reactivate_transaction(wallet, value, fee, validity_start_height)
            .await?;
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
            .await?;
        self.send_raw_transaction(raw_tx).await
    }

    async fn create_new_validator_transaction(
        &mut self,
        wallet: Address,
        warm_key: Address,
        validator_secret_key: String,
        reward_address: Address,
        signal_data: String,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<String, Error> {
        let secret_key =
            BlsSecretKey::deserialize_from_vec(&hex::decode(validator_secret_key).unwrap())
                .unwrap();
        let validator_keypair = BlsKeyPair::from(secret_key);

        // Since JSON doesn't have a primitive for Option (it just has the null primitive), we can't
        // have a double Option. This becomes an issue when creating an update_validator transaction.
        // Instead we use the following work-around. We define the empty String to be None. So, in
        // this situation we have:
        // "" = None
        // "0x29a4b..." = Some(hash)
        let signal_data: Option<Blake2bHash> = if signal_data.is_empty() {
            None
        } else {
            Some(Blake2bHash::deserialize_from_vec(&hex::decode(signal_data).unwrap()).unwrap())
        };

        let transaction = TransactionBuilder::new_create_validator(
            &self.get_wallet_keypair(&wallet)?,
            &self.get_wallet_keypair(&wallet)?,
            warm_key,
            &validator_keypair,
            reward_address,
            signal_data,
            fee,
            self.validity_start_height(validity_start_height),
            self.network_id(),
        );

        Ok(transaction_to_hex_string(&transaction))
    }

    async fn send_new_validator_transaction(
        &mut self,
        wallet: Address,
        warm_key: Address,
        validator_secret_key: String,
        reward_address: Address,
        signal_data: String,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<Blake2bHash, Error> {
        let raw_tx = self
            .create_new_validator_transaction(
                wallet,
                warm_key,
                validator_secret_key,
                reward_address,
                signal_data,
                fee,
                validity_start_height,
            )
            .await?;
        self.send_raw_transaction(raw_tx).await
    }

    async fn create_update_validator_transaction(
        &mut self,
        wallet: Address,
        new_warm_address: Option<Address>,
        new_validator_secret_key: Option<String>,
        new_reward_address: Option<Address>,
        new_signal_data: Option<String>,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<String, Error> {
        let new_validator_keypair = match new_validator_secret_key {
            Some(key) => {
                let new_secret_key =
                    BlsSecretKey::deserialize_from_vec(&hex::decode(key).unwrap()).unwrap();
                Some(BlsKeyPair::from(new_secret_key))
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
                        Blake2bHash::deserialize_from_vec(&hex::decode(string).unwrap()).unwrap(),
                    ))
                }
            }
        };

        let transaction = TransactionBuilder::new_update_validator(
            &self.get_wallet_keypair(&wallet)?,
            &self.get_wallet_keypair(&wallet)?,
            new_warm_address,
            new_validator_keypair.as_ref(),
            new_reward_address,
            new_signal_data,
            fee,
            self.validity_start_height(validity_start_height),
            self.network_id(),
        );

        Ok(transaction_to_hex_string(&transaction))
    }

    async fn send_update_validator_transaction(
        &mut self,
        wallet: Address,
        new_warm_address: Option<Address>,
        new_validator_secret_key: Option<String>,
        new_reward_address: Option<Address>,
        new_signal_data: Option<String>,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<Blake2bHash, Error> {
        let raw_tx = self
            .create_update_validator_transaction(
                wallet,
                new_warm_address,
                new_validator_secret_key,
                new_reward_address,
                new_signal_data,
                fee,
                validity_start_height,
            )
            .await?;
        self.send_raw_transaction(raw_tx).await
    }

    async fn create_retire_validator_transaction(
        &mut self,
        wallet: Address,
        warm_secret_key: String,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<String, Error> {
        let secret_key =
            PrivateKey::deserialize_from_vec(&hex::decode(warm_secret_key).unwrap()).unwrap();
        let warm_key_pair = KeyPair::from(secret_key);

        let transaction = TransactionBuilder::new_retire_validator(
            &self.get_wallet_keypair(&wallet)?,
            wallet,
            &warm_key_pair,
            fee,
            self.validity_start_height(validity_start_height),
            self.network_id(),
        );

        Ok(transaction_to_hex_string(&transaction))
    }

    async fn send_retire_validator_transaction(
        &mut self,
        wallet: Address,
        warm_secret_key: String,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<Blake2bHash, Error> {
        let raw_tx = self
            .create_retire_validator_transaction(
                wallet,
                warm_secret_key,
                fee,
                validity_start_height,
            )
            .await?;
        self.send_raw_transaction(raw_tx).await
    }

    async fn create_reactivate_validator_transaction(
        &mut self,
        wallet: Address,
        warm_secret_key: String,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<String, Error> {
        let secret_key =
            PrivateKey::deserialize_from_vec(&hex::decode(warm_secret_key).unwrap()).unwrap();
        let warm_key_pair = KeyPair::from(secret_key);

        let transaction = TransactionBuilder::new_reactivate_validator(
            &self.get_wallet_keypair(&wallet)?,
            wallet,
            &warm_key_pair,
            fee,
            self.validity_start_height(validity_start_height),
            self.network_id(),
        );

        Ok(transaction_to_hex_string(&transaction))
    }

    async fn send_reactivate_validator_transaction(
        &mut self,
        wallet: Address,
        warm_secret_key: String,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<Blake2bHash, Error> {
        let raw_tx = self
            .create_reactivate_validator_transaction(
                wallet,
                warm_secret_key,
                fee,
                validity_start_height,
            )
            .await?;
        self.send_raw_transaction(raw_tx).await
    }

    async fn create_unpark_validator_transaction(
        &mut self,
        wallet: Address,
        warm_secret_key: String,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<String, Error> {
        let secret_key =
            PrivateKey::deserialize_from_vec(&hex::decode(warm_secret_key).unwrap()).unwrap();
        let warm_key_pair = KeyPair::from(secret_key);

        let transaction = TransactionBuilder::new_unpark_validator(
            &self.get_wallet_keypair(&wallet)?,
            wallet,
            &warm_key_pair,
            fee,
            self.validity_start_height(validity_start_height),
            self.network_id(),
        );

        Ok(transaction_to_hex_string(&transaction))
    }

    async fn send_unpark_validator_transaction(
        &mut self,
        wallet: Address,
        warm_secret_key: String,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<Blake2bHash, Error> {
        let raw_tx = self
            .create_unpark_validator_transaction(
                wallet,
                warm_secret_key,
                fee,
                validity_start_height,
            )
            .await?;
        self.send_raw_transaction(raw_tx).await
    }

    async fn create_drop_validator_transaction(
        &mut self,
        wallet: Address,
        recipient: Address,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<String, Error> {
        let transaction = TransactionBuilder::new_drop_validator(
            recipient,
            &self.get_wallet_keypair(&wallet)?,
            fee,
            self.validity_start_height(validity_start_height),
            self.network_id(),
        );

        Ok(transaction_to_hex_string(&transaction))
    }

    async fn send_drop_validator_transaction(
        &mut self,
        wallet: Address,
        recipient: Address,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<Blake2bHash, Error> {
        let raw_tx = self
            .create_drop_validator_transaction(wallet, recipient, fee, validity_start_height)
            .await?;
        self.send_raw_transaction(raw_tx).await
    }
}
