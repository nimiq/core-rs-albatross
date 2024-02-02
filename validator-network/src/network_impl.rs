use std::{collections::BTreeMap, error::Error, fmt::Debug, sync::Arc};

use async_trait::async_trait;
use futures::{stream::BoxStream, StreamExt, TryFutureExt};
use log::warn;
use nimiq_bls::{lazy::LazyPublicKey, CompressedPublicKey, KeyPair, SecretKey};
use nimiq_network_interface::{
    network::{CloseReason, MsgAcceptance, Network, SubscribeEvents, Topic},
    request::{Message, Request, RequestCommon},
};
use nimiq_serde::{Deserialize, Serialize};
use parking_lot::RwLock;
use time::OffsetDateTime;

use super::{MessageStream, NetworkError, PubsubId, ValidatorNetwork};
use crate::validator_record::ValidatorRecord;

/// Validator `PeerId` cache state
enum CacheState<TPeerId, TNetworkError> {
    /// Cache entry has been resolved with the peer ID
    Resolved(TPeerId),
    /// Cache entry could not have been resolved
    Error(TNetworkError),
    /// Cache entry resolution is in progress (and result is yet unknown)
    InProgress,
}

/// Validator Network implementation
pub struct ValidatorNetworkImpl<N>
where
    N: Network,
    N::PeerId: Serialize + Deserialize,
{
    /// A reference to the network containing all peers
    network: Arc<N>,
    /// Own validator ID if active, `None` otherwise.
    own_validator_id: Arc<RwLock<Option<u16>>>,
    /// Set of public keys for each of the validators
    validator_keys: Arc<RwLock<Vec<LazyPublicKey>>>,
    /// Cache for mapping validator public keys to peer IDs
    validator_peer_id_cache:
        Arc<RwLock<BTreeMap<CompressedPublicKey, CacheState<N::PeerId, NetworkError<N::Error>>>>>,
}

impl<N> ValidatorNetworkImpl<N>
where
    N: Network,
    N::PeerId: Serialize + Deserialize,
    N::Error: Sync + Send,
{
    pub fn new(network: Arc<N>) -> Self {
        Self {
            network,
            own_validator_id: Arc::new(RwLock::new(None)),
            validator_keys: Arc::new(RwLock::new(vec![])),
            validator_peer_id_cache: Arc::new(RwLock::new(BTreeMap::new())),
        }
    }

    /// For use in closures, so that no reference to `self` needs to be kept around.
    fn arc_clone(&self) -> ValidatorNetworkImpl<N> {
        ValidatorNetworkImpl {
            network: Arc::clone(&self.network),
            own_validator_id: Arc::clone(&self.own_validator_id),
            validator_keys: Arc::clone(&self.validator_keys),
            validator_peer_id_cache: Arc::clone(&self.validator_peer_id_cache),
        }
    }

    /// Returns the local validator ID, if elected, `Err(NotElected)` otherwise.
    fn local_validator_id<T: Error + Sync + 'static>(&self) -> Result<u16, NetworkError<T>> {
        self.own_validator_id.read().ok_or(NetworkError::NotElected)
    }

    /// Looks up the peer ID for a validator public key in the DHT.
    async fn resolve_peer_id(
        network: &N,
        public_key: &LazyPublicKey,
    ) -> Result<Option<N::PeerId>, NetworkError<N::Error>> {
        if let Some(record) = network
            .dht_get::<_, ValidatorRecord<N::PeerId>, KeyPair>(public_key.compressed())
            .await?
        {
            Ok(Some(record.peer_id))
        } else {
            Ok(None)
        }
    }

    // Looks up the peer ID for a validator public key in the DHT and updates the internal cache
    async fn update_peer_id_cache(&self, validator_id: u16, public_key: &LazyPublicKey) {
        let cache_value = match Self::resolve_peer_id(&self.network, public_key).await {
            Ok(result) => match result {
                Some(peer_id) => {
                    log::trace!(
                        %peer_id,
                        validator_id,
                        %public_key,
                        "Resolved validator peer ID"
                    );
                    CacheState::Resolved(peer_id)
                }
                None => {
                    log::trace!(validator_id, %public_key, "Unable to resolve validator peer ID: Entry not found in DHT");
                    CacheState::Error(NetworkError::Unreachable)
                }
            },
            Err(e) => {
                log::trace!(
                    validator_id,
                    error = ?e,
                    %public_key,
                    "Unable to resolve validator peer ID: Network error"
                );
                CacheState::Error(e)
            }
        };
        self.validator_peer_id_cache
            .write()
            .insert(public_key.compressed().clone(), cache_value);
    }

    /// Look up the peer ID for a validator ID.
    fn get_validator_peer_id(
        &self,
        validator_id: u16,
    ) -> Result<N::PeerId, NetworkError<N::Error>> {
        let public_key = self
            .validator_keys
            .read()
            .get(usize::from(validator_id))
            .ok_or(NetworkError::UnknownValidator(validator_id))?
            .clone();

        let update_peer_cache = {
            if let Some(cache_state) = self
                .validator_peer_id_cache
                .read()
                .get(&public_key.compressed().clone())
            {
                match cache_state {
                    CacheState::Resolved(peer_id) => return Ok(*peer_id),
                    CacheState::Error(_) => true,
                    CacheState::InProgress => {
                        log::debug!(validator_id, "Record resolution is in progress");
                        false
                    }
                }
            } else {
                log::debug!(
                    ?public_key,
                    validator_id,
                    "Could not find peer ID for validator in DHT. Performing a DHT query",
                );
                true
            }
        };
        if update_peer_cache {
            // Cache is empty for this validator ID, query the entry
            {
                // Re-check the validator Peer ID cache with the write lock taken and update it if necessary
                let mut validator_peer_id_cache = self.validator_peer_id_cache.write();
                if let Some(cache_state) =
                    validator_peer_id_cache.get(&public_key.compressed().clone())
                {
                    match cache_state {
                        CacheState::Resolved(peer_id) => return Ok(*peer_id),
                        CacheState::Error(_) => {
                            validator_peer_id_cache
                                .insert(public_key.compressed().clone(), CacheState::InProgress);
                        }
                        CacheState::InProgress => {
                            log::debug!(validator_id, "Record resolution is in progress");
                            return Err(NetworkError::UnknownValidator(validator_id));
                        }
                    }
                }
            }
            let self_ = self.arc_clone();
            tokio::spawn(async move {
                Self::update_peer_id_cache(&self_, validator_id, &public_key).await;
            });
        }
        Err(NetworkError::UnknownValidator(validator_id))
    }

    /// Clears the validator->peer_id cache.
    /// The cached entry should be cleared when the peer id might have changed.
    fn clear_validator_peer_id_cache(&self, validator_id: u16) {
        if let Some(validator_key) = self.validator_keys.read().get(usize::from(validator_id)) {
            self.validator_peer_id_cache
                .write()
                .remove(validator_key.compressed());
        }
    }
}

/// Messages sent over the validator network get augmented with the sending
/// validator's ID.
///
/// This makes it easier for the recipient to check that the sender is indeed a
/// currently elected validator.
#[derive(Debug, Deserialize, Serialize)]
struct ValidatorMessage<M> {
    validator_id: u16,
    inner: M,
}

impl<M: RequestCommon> RequestCommon for ValidatorMessage<M> {
    type Kind = M::Kind;
    // Use distinct type IDs for the validator network.
    const TYPE_ID: u16 = 10_000 + M::TYPE_ID;
    type Response = M::Response;
    const MAX_REQUESTS: u32 = M::MAX_REQUESTS;
}

// Proposal - gossip
// LevelUpdate - multicast
// StateEx - request/response

#[async_trait]
impl<N> ValidatorNetwork for ValidatorNetworkImpl<N>
where
    N: Network,
    N::PeerId: Serialize + Deserialize,
    N::Error: Send,
    <N as Network>::Error: Sync,
{
    type Error = NetworkError<N::Error>;
    type NetworkType = N;

    fn set_validator_id(&self, validator_id: Option<u16>) {
        *self.own_validator_id.write() = validator_id;
    }

    /// Tells the validator network the validator keys for the current set of active validators. The keys must be
    /// ordered, such that the k-th entry is the validator with ID k.
    async fn set_validators(&self, validator_keys: Vec<LazyPublicKey>) {
        log::trace!(?validator_keys, "Setting validators for ValidatorNetwork");
        // Create new peer ID cache, but keep validators that are still active.
        let mut validator_peer_id_cache = self.validator_peer_id_cache.write();
        let mut keep_cached = BTreeMap::new();

        for validator_key in &validator_keys {
            if let Some(peer_id) = validator_peer_id_cache.remove(validator_key.compressed()) {
                keep_cached.insert(validator_key.compressed().clone(), peer_id);
            }
        }

        *self.validator_keys.write() = validator_keys;
        *validator_peer_id_cache = keep_cached;
    }

    async fn send_to<M: Message>(&self, validator_id: u16, msg: M) -> Result<(), Self::Error> {
        let msg = ValidatorMessage {
            validator_id: self.local_validator_id()?,
            inner: msg,
        };
        let peer_id = self.get_validator_peer_id(validator_id).map_err(|error| {
            log::error!(?error, "Error getting validator peer ID");
            error
        })?;

        self.network
            .message(msg, peer_id)
            .map_err(|e| {
                // The validator peer id might have changed and thus caused a connection failure.
                self.clear_validator_peer_id_cache(validator_id);

                NetworkError::Request(e)
            })
            .await
    }

    async fn request<TRequest: Request>(
        &self,
        request: TRequest,
        validator_id: u16,
    ) -> Result<
        <TRequest as RequestCommon>::Response,
        NetworkError<<Self::NetworkType as Network>::Error>,
    > {
        let request = ValidatorMessage {
            validator_id: self.local_validator_id()?,
            inner: request,
        };
        if let Ok(peer_id) = self.get_validator_peer_id(validator_id) {
            self.network
                .request(request, peer_id)
                .map_err(|e| {
                    // The validator peer id might have changed and thus caused a connection failure.
                    self.clear_validator_peer_id_cache(validator_id);

                    NetworkError::Request(e)
                })
                .await
        } else {
            Err(NetworkError::Unreachable)
        }
    }

    fn receive<M>(&self) -> MessageStream<M>
    where
        M: Message + Clone,
    {
        let self_ = self.arc_clone();
        Box::pin(
            self.network
                .receive_messages::<ValidatorMessage<M>>()
                .filter_map(move |(message, peer_id)| {
                    let self_ = self_.arc_clone();
                    async move {
                        let validator_peer_id = self_.get_validator_peer_id(message.validator_id).ok();
                        // Check that each message actually comes from the peer that it
                        // claims it comes from. Reject it otherwise.
                        if validator_peer_id
                            .as_ref()
                            .map(|pid| *pid != peer_id)
                            .unwrap_or(true)
                        {
                            warn!(%peer_id, ?validator_peer_id, claimed_validator_id = message.validator_id, "dropping validator message");
                            return None;
                        }
                        Some((message.inner, message.validator_id as usize))
                    }
                }),
        )
    }

    async fn publish<TTopic>(&self, item: TTopic::Item) -> Result<(), Self::Error>
    where
        TTopic: Topic + Sync,
    {
        self.network.publish::<TTopic>(item).await?;
        Ok(())
    }

    async fn subscribe<'a, TTopic>(
        &self,
    ) -> Result<BoxStream<'a, (TTopic::Item, PubsubId<Self>)>, Self::Error>
    where
        TTopic: Topic + Sync,
    {
        Ok(self.network.subscribe::<TTopic>().await?)
    }

    fn subscribe_events(&self) -> SubscribeEvents<<Self::NetworkType as Network>::PeerId> {
        self.network.subscribe_events()
    }

    async fn set_public_key(
        &self,
        public_key: &CompressedPublicKey,
        secret_key: &SecretKey,
    ) -> Result<(), Self::Error> {
        let peer_id = self.network.get_local_peer_id();
        let record = ValidatorRecord::new(
            peer_id,
            (OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000) as u64,
        );
        self.network
            .dht_put(public_key, &record, &KeyPair::from(*secret_key))
            .await?;

        Ok(())
    }

    async fn disconnect_peer(&self, peer_id: N::PeerId, close_reason: CloseReason) {
        self.network.disconnect_peer(peer_id, close_reason).await
    }

    fn validate_message<TTopic>(&self, id: PubsubId<Self>, acceptance: MsgAcceptance)
    where
        TTopic: Topic + Sync,
    {
        self.network.validate_message::<TTopic>(id, acceptance);
    }
}
