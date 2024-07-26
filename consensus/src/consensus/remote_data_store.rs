use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
};

use nimiq_account::{
    Account, DataStoreReadOps, Staker, StakingContract, StakingContractStore, Tombstone, Validator,
};
use nimiq_blockchain_interface::AbstractBlockchain;
use nimiq_blockchain_proxy::BlockchainProxy;
use nimiq_keys::Address;
use nimiq_network_interface::{
    network::{CloseReason, Network},
    peer_info::Services,
    request::{OutboundRequestError, RequestError},
};
use nimiq_primitives::{key_nibbles::KeyNibbles, policy::Policy};
use nimiq_serde::Deserialize;

use crate::messages::RequestTrieProof;

/// The Remote Data Store is a component to remotely request data from the staking
/// contract such as:
/// - Validators
/// - Stakers
/// - Tombstones
///
/// It also serves as an utility function to extract any type from the Accounts Trie
/// remotely.
pub(crate) struct RemoteDataStore<N: Network> {
    /// An Arc of the network to make requests.
    pub(crate) network: Arc<N>,
    /// A blockchain proxy for verifying proofs of data received.
    pub(crate) blockchain: BlockchainProxy,
    /// Minimum set of peers that request should be made to.
    pub(crate) min_peers: usize,
}

/// Internal Remote operations the Remote Data Store can perform over addresses on
/// wasm
enum RemoteDataStoreOps {
    /// Gets a set of validators by their addresses
    Validator(Vec<Address>),
    /// Gets a set of stakers by their addresses
    Staker(Vec<Address>),
    /// Gets a set of tombstones by their addresses
    Tombstone(Vec<Address>),
}

impl<N: Network> RemoteDataStore<N> {
    /// Gets a proof for a deserializable item in a remote accounts trie and returns
    /// the item if a valid proof was obtained or `None` if not.
    pub(crate) async fn get_trie<T: Deserialize>(
        network: Arc<N>,
        blockchain: BlockchainProxy,
        keys: &[KeyNibbles],
        min_peers: usize,
    ) -> Result<BTreeMap<KeyNibbles, Option<T>>, RequestError> {
        // First we tell the network to provide us with a vector that contains all the connected peers that support such services
        // Note: If the network could not provide enough peers that satisfies our requirement, then an error would be returned
        let peers = network
            .get_peers_by_services(Services::ACCOUNTS_PROOF, min_peers)
            .await
            .map_err(|error| {
                log::error!(
                    err = %error,
                    "Request items by trie keys couldn't be fulfilled"
                );
                RequestError::OutboundRequest(OutboundRequestError::SendError)
            })?;

        for peer_id in peers {
            log::debug!(
                peer_id = %peer_id,
                "Performing accounts by address request to peer",
            );
            log::debug!("Getting accounts for {:?}", keys.iter());
            let response = network
                .request::<RequestTrieProof>(
                    RequestTrieProof {
                        keys: keys.to_vec(),
                    },
                    peer_id,
                )
                .await;

            match response {
                Ok(Ok(response)) => {
                    let blockchain = blockchain.read();
                    // First try to obtain, from our chain store, the block that was used to generate the proof
                    let block = blockchain.get_block(&response.block_hash, false).ok();

                    if let Some(block) = block {
                        // Now we need to verify the proof
                        if let Ok(values) = response
                            .proof
                            .verify_values(block.state_root(), &keys.iter().collect::<Vec<_>>())
                        {
                            return Ok(values
                                .into_iter()
                                .map(|(key, value)| {
                                    (key, value.map(|v| T::deserialize_from_vec(&v).unwrap()))
                                })
                                .collect());
                        } else {
                            // If the proof does not verify, we disconnect from the peer
                            log::warn!(%peer_id, "Banning peer because the accounts proof didn't verify");
                            network
                                .disconnect_peer(peer_id, CloseReason::MaliciousPeer)
                                .await;
                            break;
                        }
                    } else {
                        // If we couldn't find the block, then we cannot verify the proof
                        // A malicious peer could just send random hashes.
                        log::debug!(block_hash = %response.block_hash, "Received an accounts proof, but we could not find the block that was used to generate the proof");
                    }
                }
                Ok(Err(error)) => {
                    // If there is no proof, then we just continue with the next peer
                    log::debug!(peer = %peer_id, %error, "We requested an accounts proof but the peer didn't provide any");
                }
                Err(error) => {
                    // If there was a request error with this peer we log an error
                    log::error!(peer = %peer_id, %error, "There was an error requesting accounts proof from peer");
                }
            }
        }

        Err(RequestError::OutboundRequest(
            OutboundRequestError::NoResponse,
        ))
    }

    async fn get_staking_contract(&self) -> Result<StakingContract, RequestError> {
        let key = KeyNibbles::from(&Policy::STAKING_CONTRACT_ADDRESS);
        let accounts = Self::get_trie(
            Arc::clone(&self.network),
            self.blockchain.clone(),
            &[key.clone()],
            self.min_peers,
        )
        .await?;

        if accounts.len() != 1 {
            log::error!(
                len = accounts.len(),
                "Expected only one account for the staking contract"
            );
            return Err(RequestError::OutboundRequest(
                OutboundRequestError::SendError,
            ));
        }

        if let Some(account) = accounts.get(&key) {
            if let Some(account) = account {
                match account {
                    Account::Staking(account) => Ok(account.clone()),
                    _ => {
                        log::error!(
                            "Requested a staking contract account and received another account type"
                        );
                        Err(RequestError::OutboundRequest(
                            OutboundRequestError::SendError,
                        ))
                    }
                }
            } else {
                log::error!("No proof was provided for the staking contract");
                Err(RequestError::OutboundRequest(
                    OutboundRequestError::SendError,
                ))
            }
        } else {
            log::error!("Returned accounts do not include staking contract");
            Err(RequestError::OutboundRequest(
                OutboundRequestError::SendError,
            ))
        }
    }

    /// Gets a set of validators given their addresses. The returned type is a
    /// BTreeMap of addresses to an optional `Validator`. If a validator was not
    /// found, then `None` is returned in its corresponding entry.
    pub(crate) async fn get_validators(
        &self,
        addresses: Vec<Address>,
    ) -> Result<BTreeMap<Address, Option<Validator>>, RequestError> {
        if cfg!(target_family = "wasm") {
            self.wasm_exec(RemoteDataStoreOps::Validator(addresses))
                .await
        } else {
            let staking_contract = self.get_staking_contract().await?;
            let mut validators = BTreeMap::new();
            for address in addresses {
                validators.insert(
                    address.clone(),
                    staking_contract.get_validator(self, &address),
                );
            }
            Ok(validators)
        }
    }

    /// Gets a set of stakers given their addresses. The returned type is a
    /// BTreeMap of addresses to an optional `Staker`. If a staker was not
    /// found, then `None` is returned in its corresponding entry.
    pub(crate) async fn get_stakers(
        &self,
        addresses: Vec<Address>,
    ) -> Result<BTreeMap<Address, Option<Staker>>, RequestError> {
        if cfg!(target_family = "wasm") {
            self.wasm_exec(RemoteDataStoreOps::Staker(addresses)).await
        } else {
            let staking_contract = self.get_staking_contract().await?;
            let mut stakers = BTreeMap::new();
            for address in addresses {
                stakers.insert(address.clone(), staking_contract.get_staker(self, &address));
            }
            Ok(stakers)
        }
    }

    /// Gets a set of tombstones given their addresses. The returned type is a
    /// BTreeMap of addresses to an optional `Tombstone`. If a tombstone was not
    /// found, then `None` is returned in its corresponding entry.
    #[allow(dead_code)]
    pub(crate) async fn get_tombstones(
        &self,
        addresses: Vec<Address>,
    ) -> Result<BTreeMap<Address, Option<Tombstone>>, RequestError> {
        if cfg!(target_family = "wasm") {
            self.wasm_exec(RemoteDataStoreOps::Tombstone(addresses))
                .await
        } else {
            let staking_contract = self.get_staking_contract().await?;
            let mut tombstones = BTreeMap::new();
            for address in addresses {
                tombstones.insert(
                    address.clone(),
                    staking_contract.get_tombstone(self, &address),
                );
            }
            Ok(tombstones)
        }
    }

    async fn wasm_exec<T: Deserialize + Clone>(
        &self,
        op: RemoteDataStoreOps,
    ) -> Result<BTreeMap<Address, Option<T>>, RequestError> {
        let staking_contract_key = KeyNibbles::from(&Policy::STAKING_CONTRACT_ADDRESS);
        let mut keys_to_address: HashMap<KeyNibbles, Address> = match op {
            RemoteDataStoreOps::Validator(addresses) => {
                HashMap::from_iter(addresses.iter().map(|address| {
                    (
                        &staking_contract_key + &StakingContractStore::validator_key(address),
                        address.clone(),
                    )
                }))
            }
            RemoteDataStoreOps::Staker(addresses) => {
                HashMap::from_iter(addresses.iter().map(|address| {
                    (
                        &staking_contract_key + &StakingContractStore::staker_key(address),
                        address.clone(),
                    )
                }))
            }
            RemoteDataStoreOps::Tombstone(addresses) => {
                HashMap::from_iter(addresses.iter().map(|address| {
                    (
                        &staking_contract_key + &StakingContractStore::tombstone_key(address),
                        address.clone(),
                    )
                }))
            }
        };

        let keys: Vec<KeyNibbles> = keys_to_address.keys().cloned().collect();

        let items = Self::get_trie::<T>(
            Arc::clone(&self.network),
            self.blockchain.clone(),
            &keys,
            self.min_peers,
        )
        .await?;

        Ok(items
            .iter()
            .map(|(key, item)| {
                (
                    keys_to_address
                        .remove(key)
                        .expect("Key should have an address"),
                    item.clone(),
                )
            })
            .collect())
    }
}

impl<N: Network> DataStoreReadOps for RemoteDataStore<N> {
    fn get<T: Deserialize>(&self, key: &KeyNibbles) -> Option<T> {
        let proof = futures_executor::block_on(Self::get_trie(
            Arc::clone(&self.network),
            self.blockchain.clone(),
            &[key.clone()],
            self.min_peers,
        ))
        .ok();
        if let Some(mut proof) = proof {
            if proof.len() != 1 {
                log::error!(len = proof.len(), "Unexpected amount of proved items");
                None
            } else {
                proof.remove(key).flatten()
            }
        } else {
            None
        }
    }
}
