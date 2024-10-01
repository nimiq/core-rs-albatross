use std::{collections::BTreeMap, error::Error, fmt::Debug, sync::Arc};

use async_trait::async_trait;
use futures::{stream::BoxStream, StreamExt, TryFutureExt};
use log::warn;
use nimiq_bls::{lazy::LazyPublicKey, CompressedPublicKey, KeyPair, SecretKey};
use nimiq_network_interface::{
    network::{CloseReason, MsgAcceptance, Network, SubscribeEvents, Topic},
    request::{InboundRequestError, Message, Request, RequestCommon, RequestError},
};
use nimiq_serde::{Deserialize, Serialize};
use nimiq_utils::spawn;
use parking_lot::RwLock;
use time::OffsetDateTime;

use super::{MessageStream, NetworkError, PubsubId, ValidatorNetwork};
use crate::validator_record::ValidatorRecord;

/// Validator `PeerId` cache state
#[derive(Clone, Copy)]
enum CacheState<TPeerId> {
    /// Cache entry has been resolved with the peer ID
    Resolved(TPeerId),
    /// Cache entry could not have been resolved.
    ///
    /// We might know a previous peer ID.
    Error(Option<TPeerId>),
    /// Cache entry resolution is in progress (and result is yet unknown).
    ///
    /// We might know a previous peer ID.
    InProgress(Option<TPeerId>),
    /// No cached peer ID, but a previous one is known.
    Empty(TPeerId),
}

impl<TPeerId: Clone> CacheState<TPeerId> {
    fn current_peer_id(&self) -> Option<TPeerId> {
        match self {
            CacheState::Resolved(peer_id) => Some(peer_id.clone()),
            _ => None,
        }
    }
    fn potentially_outdated_peer_id(&self) -> Option<TPeerId> {
        match self {
            CacheState::Resolved(peer_id) => Some(peer_id.clone()),
            CacheState::Error(maybe_peer_id) => maybe_peer_id.clone(),
            CacheState::InProgress(maybe_peer_id) => maybe_peer_id.clone(),
            CacheState::Empty(peer_id) => Some(peer_id.clone()),
        }
    }
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
    validator_peer_id_cache: Arc<RwLock<BTreeMap<CompressedPublicKey, CacheState<N::PeerId>>>>,
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

    /// Looks up the peer ID for a validator public key in the DHT and updates
    /// the internal cache.
    ///
    /// Assumes that the cache entry has been set to `InProgress` by the
    /// caller, will panic otherwise.
    async fn update_peer_id_cache(&self, validator_id: u16, public_key: &LazyPublicKey) {
        let cache_value = match Self::resolve_peer_id(&self.network, public_key).await {
            Ok(Some(peer_id)) => {
                log::trace!(
                    %peer_id,
                    validator_id,
                    %public_key,
                    "Resolved validator peer ID"
                );
                Ok(peer_id)
            }
            Ok(None) => {
                log::debug!(validator_id, %public_key, "Unable to resolve validator peer ID: Entry not found in DHT");
                Err(())
            }
            Err(error) => {
                log::debug!(
                    validator_id,
                    ?error,
                    %public_key,
                    "Unable to resolve validator peer ID: Network error"
                );
                Err(())
            }
        };

        match self
            .validator_peer_id_cache
            .write()
            .get_mut(public_key.compressed())
        {
            Some(cache_entry) => {
                if let CacheState::InProgress(prev_peer_id) = *cache_entry {
                    *cache_entry = match cache_value {
                        Ok(peer_id) => CacheState::Resolved(peer_id),
                        Err(()) => CacheState::Error(prev_peer_id),
                    };
                } else {
                    unreachable!("cache state must be \"in progress\"");
                }
            }
            None => unreachable!("cache state must exist"),
        }
    }

    /// Look up the peer ID for a validator ID.
    fn get_validator_cache(&self, validator_id: u16) -> CacheState<N::PeerId> {
        let public_key = match self.validator_keys.read().get(usize::from(validator_id)) {
            Some(pk) => pk.clone(),
            None => return CacheState::Error(None),
        };

        if let Some(cache_state) = self
            .validator_peer_id_cache
            .read()
            .get(public_key.compressed())
        {
            match *cache_state {
                CacheState::Resolved(..) => return *cache_state,
                CacheState::Error(..) => {}
                CacheState::InProgress(..) => {
                    log::trace!(validator_id, "Record resolution is in progress");
                    return *cache_state;
                }
                CacheState::Empty(..) => {}
            }
        }

        let new_cache_state;
        // Cache is empty for this validator ID, query the entry
        {
            // Re-check the validator Peer ID cache with the write lock taken and update it if necessary
            let mut validator_peer_id_cache = self.validator_peer_id_cache.write();
            if let Some(cache_state) = validator_peer_id_cache.get_mut(public_key.compressed()) {
                new_cache_state = match *cache_state {
                    CacheState::Resolved(..) => return *cache_state,
                    CacheState::Error(prev_peer_id) => {
                        log::debug!(validator_id, "Record resolution failed. Retrying...");
                        CacheState::InProgress(prev_peer_id)
                    }
                    CacheState::InProgress(..) => {
                        log::trace!(validator_id, "Record resolution is in progress");
                        return *cache_state;
                    }
                    CacheState::Empty(prev_peer_id) => {
                        log::debug!(validator_id, "Cache entry was emptied, re-querying DHT...");
                        CacheState::InProgress(Some(prev_peer_id))
                    }
                };
                *cache_state = new_cache_state;
            } else {
                new_cache_state = CacheState::InProgress(None);
                // No cache entry for this validator ID: we are going to perform the DHT query
                validator_peer_id_cache.insert(public_key.compressed().clone(), new_cache_state);
                log::debug!(
                    ?public_key,
                    validator_id,
                    "No cache entry found, querying DHT",
                );
            }
        }
        let self_ = self.arc_clone();
        spawn(async move {
            Self::update_peer_id_cache(&self_, validator_id, &public_key).await;
        });
        new_cache_state
    }

    /// Clears the validator->peer_id cache on a `RequestError`.
    /// The cached entry should be cleared when the peer id might have changed.
    fn clear_validator_peer_id_cache_on_error(
        &self,
        validator_id: u16,
        error: &RequestError,
        peer_id: &N::PeerId,
    ) {
        match error {
            // The no receiver is not an error since the peer might not be aggregating
            RequestError::InboundRequest(InboundRequestError::NoReceiver) => {}
            // In all other cases, clear the Peer ID cache for this validator
            _ => {
                // Make sure to drop the `self.validator_keys` lock after this line.
                let validator_key = self
                    .validator_keys
                    .read()
                    .get(usize::from(validator_id))
                    .cloned();
                if let Some(validator_key) = validator_key {
                    // Clear the peer ID cache only if the error happened for the same Peer ID that we have cached
                    let mut validator_peer_id_cache = self.validator_peer_id_cache.write();
                    if let Some(cache_entry) =
                        validator_peer_id_cache.get_mut(validator_key.compressed())
                    {
                        if let CacheState::Resolved(cached_peer_id) = *cache_entry {
                            if cached_peer_id == *peer_id {
                                *cache_entry = CacheState::Empty(cached_peer_id);
                            }
                        }
                    }
                }
            }
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

        // Put the `validator_keys` into the same order as the
        // `self.validator_peer_id_cache` so that we can simultaneously iterate
        // over them. Note that `LazyPublicKey::cmp` forwards to
        // `CompressedPublicKey::cmp`.
        let mut sorted_validator_keys: Vec<_> = validator_keys.iter().collect();
        sorted_validator_keys.sort_unstable();
        let mut sorted_validator_keys = sorted_validator_keys.into_iter();
        let mut cur_key = sorted_validator_keys.next();

        // Drop peer ID cache, but keep validators that are still active and
        // validators who are currently being resolved.
        self.validator_peer_id_cache
            .write()
            .retain(|key, cache_state| {
                // If a lookup is in progress, the lookup thread expects to be
                // able to put the result into the cache map.
                //
                // It'll get cleaned up on the next validator change.
                if let CacheState::InProgress(..) = cache_state {
                    return true;
                }
                // Move `cur_key` until we're greater or equal to `key`.
                while cur_key.map(|k| k.compressed() < key).unwrap_or(false) {
                    cur_key = sorted_validator_keys.next();
                }
                Some(key) == cur_key.map(LazyPublicKey::compressed)
            });

        *self.validator_keys.write() = validator_keys;
    }

    async fn send_to<M: Message>(&self, validator_id: u16, msg: M) -> Result<(), Self::Error> {
        let msg = ValidatorMessage {
            validator_id: self.local_validator_id()?,
            inner: msg,
        };
        // Use the last known peer ID, knowing that it might be already outdated.
        // The network doesn't have a way to know if a record is outdated but we mark
        // them as potentially outdated when a request/response error happens.
        // If the cache has a potentially outdated value, it will be updated soon
        // and then available to use by future calls to this function.
        let peer_id = self
            .get_validator_cache(validator_id)
            .potentially_outdated_peer_id()
            .ok_or_else(|| NetworkError::UnknownValidator(validator_id))?;

        self.network
            .message(msg, peer_id)
            .map_err(|e| {
                // The validator peer id might have changed and thus caused a connection failure.
                self.clear_validator_peer_id_cache_on_error(validator_id, &e, &peer_id);

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
        if let Some(peer_id) = self.get_validator_cache(validator_id).current_peer_id() {
            self.network
                .request(request, peer_id)
                .map_err(|e| {
                    // The validator peer id might have changed and thus caused a connection failure.
                    self.clear_validator_peer_id_cache_on_error(validator_id, &e, &peer_id);

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
                        let validator_peer_id = self_.get_validator_cache(message.validator_id).potentially_outdated_peer_id();
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
                        Some((message.inner, message.validator_id))
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
