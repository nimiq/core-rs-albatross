use std::{collections::BTreeMap, error::Error, fmt::Debug, future, sync::Arc};

use async_trait::async_trait;
use futures::{future::BoxFuture, stream::BoxStream, FutureExt, StreamExt, TryFutureExt};
use log::warn;
use nimiq_keys::{Address, KeyPair};
use nimiq_network_interface::{
    network::{CloseReason, MsgAcceptance, Network, SubscribeEvents, Topic},
    request::{InboundRequestError, Message, Request, RequestCommon, RequestError},
};
use nimiq_primitives::slots_allocation::{Validator, Validators};
use nimiq_serde::{Deserialize, Serialize};
use nimiq_utils::spawn;
use parking_lot::RwLock;
use time::OffsetDateTime;

use super::{MessageStream, NetworkError, PubsubId, ValidatorNetwork};
use crate::{validator_claim::ValidatorClaimSigner, validator_record::ValidatorRecord};

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

pub type DhtFallback<N> =
    dyn Fn(Address) -> BoxFuture<'static, Option<<N as Network>::PeerId>> + Send + Sync;

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
    /// Per validator_id contains the validator_address for each of the validators
    validators: Arc<RwLock<Option<Validators>>>,
    /// Cache for mapping validator public keys to peer IDs
    validator_peer_id_cache: Arc<RwLock<BTreeMap<Address, CacheState<N::PeerId>>>>,
    dht_fallback: Arc<DhtFallback<N>>,
}

impl<N> ValidatorNetworkImpl<N>
where
    N: Network,
    N::PeerId: Serialize + Deserialize,
    N::Error: Sync + Send,
{
    pub fn new(network: Arc<N>) -> Self {
        Self::new_with_fallback(network, Arc::new(|_| future::ready(None).boxed()))
    }

    pub fn new_with_fallback(network: Arc<N>, dht_fallback: Arc<DhtFallback<N>>) -> Self {
        Self {
            network,
            own_validator_id: Arc::new(RwLock::new(None)),
            validators: Arc::new(RwLock::new(None)),
            validator_peer_id_cache: Arc::new(RwLock::new(BTreeMap::new())),
            dht_fallback,
        }
    }

    /// For use in closures, so that no reference to `self` needs to be kept around.
    fn arc_clone(&self) -> ValidatorNetworkImpl<N> {
        ValidatorNetworkImpl {
            network: Arc::clone(&self.network),
            own_validator_id: Arc::clone(&self.own_validator_id),
            validators: Arc::clone(&self.validators),
            validator_peer_id_cache: Arc::clone(&self.validator_peer_id_cache),
            dht_fallback: Arc::clone(&self.dht_fallback),
        }
    }

    /// Returns the local validator ID, if elected, `Err(NotElected)` otherwise.
    fn local_validator_id<T: Error + Sync + 'static>(&self) -> Result<u16, NetworkError<T>> {
        self.own_validator_id.read().ok_or(NetworkError::NotElected)
    }

    /// Given the Validators and a validator_id, returns the Validator represented by the id if it exists.
    /// None otherwise.
    fn get_validator(validators: Option<&Validators>, validator_id: u16) -> Option<&Validator> {
        // Acquire read on the validators and make sure they have been set. Return None otherwise.
        validators.and_then(|validators| validators.get_validator_by_slot_band(validator_id))
    }

    /// Looks up the peer ID for a validator address.
    ///
    /// Sources are tried cheapest first, and signed ones before the unsigned HTTPS fallback. A
    /// peer we are already connected to, whose claim to be this validator we verified in one of
    /// its peer contacts that has not aged out yet, answers immediately, while a DHT lookup can
    /// take seconds or time out entirely. A contact for a peer we are *not* connected to is only
    /// consulted after the DHT, so that a contact left behind by a validator that moved does not
    /// win over the record the validator published itself. The HTTPS fallback is only asked when
    /// neither the DHT nor the validator's newest claim has an answer other than `exclude`: it is
    /// signed by nobody, so it must not override what the validator signed.
    ///
    /// `exclude` is the peer ID we used for this validator before, when a request to it failed.
    /// Any source pointing to it is passed over, so a DHT record that points to it loses to an
    /// unconnected contact for another peer: the record may be stale because the validator moved.
    /// As the failure may just as well have been transient, `exclude` is still returned if no
    /// other peer is found and one of the sources points to it.
    async fn resolve_peer_id(
        network: &N,
        validator_address: &Address,
        fallback: Arc<DhtFallback<N>>,
        exclude: Option<N::PeerId>,
    ) -> Result<Option<N::PeerId>, NetworkError<N::Error>> {
        if let Some(peer_id) =
            Self::resolve_peer_id_contact_book(network, validator_address, exclude, true)
        {
            Self::log_resolved(
                peer_id,
                validator_address,
                exclude,
                "a connected peer contact",
            );
            return Ok(Some(peer_id));
        }

        let result = Self::resolve_peer_id_dht(network, validator_address).await;
        let dht_peer_id = result.as_ref().ok().and_then(|peer_id| *peer_id);
        if let Some(peer_id) = dht_peer_id
            && Some(peer_id) != exclude
        {
            Self::log_resolved(peer_id, validator_address, exclude, "the DHT");
            return Ok(Some(peer_id));
        }

        if let Some(peer_id) =
            Self::resolve_peer_id_contact_book(network, validator_address, exclude, false)
        {
            Self::log_resolved(peer_id, validator_address, exclude, "a peer contact");
            return Ok(Some(peer_id));
        }

        // The unsigned HTTPS fallback must not override the record the validator signed, not even
        // one that points to the peer that just failed, nor its newest claim, unless that claim
        // points to the peer that just failed: a claim is left behind when the validator moves,
        // while its record is replaced, so a claim to a failed peer is more likely to be stale.
        let fallback_peer_id = if dht_peer_id.is_none() {
            fallback(validator_address.clone()).await
        } else {
            None
        };
        if let Some(peer_id) = fallback_peer_id
            && Some(peer_id) != exclude
        {
            Self::log_resolved(peer_id, validator_address, exclude, "the DHT fallback");
            return Ok(Some(peer_id));
        }

        // Nothing points elsewhere. If a source still points to the peer that just failed, the
        // failure may have been transient, so try it again rather than giving up on the validator.
        if let Some(peer_id) = exclude
            && (dht_peer_id == Some(peer_id)
                || fallback_peer_id == Some(peer_id)
                || network
                    .get_validator_peer_ids(validator_address)
                    .contains(&peer_id))
        {
            log::debug!(%peer_id, %validator_address, "Resolved validator peer ID to the peer that failed before, no source points elsewhere");
            return Ok(Some(peer_id));
        }

        result
    }

    /// Logs where the peer ID of a validator was resolved from.
    ///
    /// Resolving a validator to another peer ID than the one we used before is logged at info
    /// level. It means that the validator moved, that the peer ID we had was wrong, or that the
    /// peer we used failed and another peer claiming to be the validator is tried instead. `source`
    /// tells which.
    fn log_resolved(
        peer_id: N::PeerId,
        validator_address: &Address,
        previous_peer_id: Option<N::PeerId>,
        source: &str,
    ) {
        if let Some(previous_peer_id) = previous_peer_id
            && previous_peer_id != peer_id
        {
            log::info!(%peer_id, %previous_peer_id, %validator_address, source, "Resolved validator to a different peer ID");
        } else {
            log::debug!(%peer_id, %validator_address, source, "Resolved validator peer ID");
        }
    }

    /// Looks up the peer ID for a validator address in the peer contact book.
    ///
    /// Only the peer with the validator's newest claim that we verified in a peer contact that has
    /// not aged out yet is considered. Any other is a node the validator left behind: even while
    /// still connected, it would take precedence over the validator's newer DHT record, and its
    /// messages would not even be heard (see `accept_sender`). Trying such nodes one after the
    /// other, `exclude` remembering just the last of them, could also keep us from ever asking the
    /// remaining sources. With `connected_only`, the peer must already be connected, which makes
    /// the answer both instant and known to be reachable.
    fn resolve_peer_id_contact_book(
        network: &N,
        validator_address: &Address,
        exclude: Option<N::PeerId>,
        connected_only: bool,
    ) -> Option<N::PeerId> {
        network
            .get_validator_peer_ids(validator_address)
            .into_iter()
            .next()
            .filter(|peer_id| Some(*peer_id) != exclude)
            .filter(|peer_id| !connected_only || network.has_peer(*peer_id))
    }

    async fn resolve_peer_id_dht(
        network: &N,
        validator_address: &Address,
    ) -> Result<Option<N::PeerId>, NetworkError<N::Error>> {
        if let Some(record) = network
            .dht_get::<_, ValidatorRecord<N::PeerId>, KeyPair>(validator_address)
            .await?
        {
            Ok(Some(record.peer_id))
        } else {
            Ok(None)
        }
    }

    /// Resolves the peer ID for a validator address (see `resolve_peer_id`) and
    /// updates the internal cache.
    ///
    /// Assumes that the cache entry has been set to `InProgress` by the
    /// caller, will panic otherwise.
    ///
    /// The given `validator_id` is used for logging purposes only.
    async fn update_peer_id_cache(
        &self,
        validator_id: u16,
        validator_address: &Address,
        exclude: Option<N::PeerId>,
    ) {
        let cache_value = match Self::resolve_peer_id(
            &self.network,
            validator_address,
            Arc::clone(&self.dht_fallback),
            exclude,
        )
        .await
        {
            Ok(Some(peer_id)) => {
                log::trace!(
                    %peer_id,
                    validator_id,
                    %validator_address,
                    "Resolved validator peer ID"
                );
                Ok(peer_id)
            }
            Ok(None) => {
                log::debug!(validator_id, %validator_address, "Unable to resolve validator peer ID: Not found in the DHT nor in the peer contact book");
                Err(())
            }
            Err(error) => {
                log::debug!(
                    validator_id,
                    ?error,
                    %validator_address,
                    "Unable to resolve validator peer ID: Network error"
                );
                Err(())
            }
        };

        match self
            .validator_peer_id_cache
            .write()
            .get_mut(validator_address)
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
        let validators = self.validators.read();
        let Some(validator) = Self::get_validator(validators.as_ref(), validator_id) else {
            return CacheState::Error(None);
        };

        if let Some(cache_state) = self.validator_peer_id_cache.read().get(&validator.address) {
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
            if let Some(cache_state) = validator_peer_id_cache.get_mut(&validator.address) {
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
                validator_peer_id_cache.insert(validator.address.clone(), new_cache_state);
                log::debug!(
                    ?validator.address,
                    validator_id,
                    "No cache entry found, querying DHT",
                );
            }
        }

        let self_ = self.arc_clone();
        let validator_address = validator.address.clone();
        // Prefer any other peer ID over the one we just failed to reach.
        let exclude = new_cache_state.potentially_outdated_peer_id();
        spawn(async move {
            Self::update_peer_id_cache(&self_, validator_id, &validator_address, exclude).await;
        });
        new_cache_state
    }

    /// The address of the validator occupying `validator_id` in the current epoch.
    fn validator_address(&self, validator_id: u16) -> Option<Address> {
        let validators = self.validators.read();
        Self::get_validator(validators.as_ref(), validator_id)
            .map(|validator| validator.address.clone())
    }

    /// Whether `peer_id` holds the newest live verified claim to be the validator at
    /// `validator_id` (see [`Network::get_validator_peer_ids`]).
    ///
    /// Such a peer is heard as the validator besides its cached peer, so that a validator that
    /// moved to another node is not ignored until our cache catches up. Only the newest claim
    /// counts: a peer the validator left behind is not heard once the validator claimed another
    /// one. This does not change the cache, so it cannot redirect what we send to the validator.
    fn is_newest_verified_peer(&self, validator_id: u16, peer_id: N::PeerId) -> bool {
        let Some(validator_address) = self.validator_address(validator_id) else {
            return false;
        };
        // Note that the validators lock is released before we ask the network, so that the
        // contact book lock is never taken while holding it. At most
        // `PeerContactBook::MAX_BINDINGS_PER_VALIDATOR` peers are returned, so this is cheap.
        self.network
            .get_validator_peer_ids(&validator_address)
            .into_iter()
            .next()
            == Some(peer_id)
    }

    /// Checks that a message or request that claims to come from the validator at `validator_id`
    /// comes from its cached peer or from the peer with its newest verified claim.
    fn accept_sender(&self, validator_id: u16, peer_id: N::PeerId) -> bool {
        let validator_peer_id = self
            .get_validator_cache(validator_id)
            .potentially_outdated_peer_id();
        validator_peer_id == Some(peer_id) || self.is_newest_verified_peer(validator_id, peer_id)
    }

    /// Clears the validator->peer_id cache on a `RequestError`.
    /// The cached entry should be cleared when the peer id might have changed.
    fn clear_validator_peer_id_cache_on_error(
        &self,
        validator_id: u16,
        error: &RequestError,
        peer_id: &N::PeerId,
    ) {
        // The no receiver is not an error since the peer might not be aggregating.
        if *error == RequestError::InboundRequest(InboundRequestError::NoReceiver) {
            return;
        }

        // Fetch the validator from the validators. If it does not exist that peer_id is not
        // assigned in this epoch and there is no cached entry to clear.
        let validators = self.validators.read();
        let Some(validator) = Self::get_validator(validators.as_ref(), validator_id) else {
            return;
        };

        // Fetch the cache. If it does not exist there is no need to clear.
        let mut validator_peer_id_cache = self.validator_peer_id_cache.write();
        let Some(cache_entry) = validator_peer_id_cache.get_mut(&validator.address) else {
            return;
        };

        // Clear the peer ID cache only if the error happened for the same Peer ID that we have cached.
        if let CacheState::Resolved(cached_peer_id) = *cache_entry
            && cached_peer_id == *peer_id
        {
            *cache_entry = CacheState::Empty(cached_peer_id);
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
    type Response = M::Response;
    // Use distinct type IDs for the validator network.
    const TYPE_ID: u16 = 10_000 + M::TYPE_ID;
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

    fn set_validators(&self, validators: &Validators) {
        log::trace!(?validators, "Setting validators for ValidatorNetwork");

        // Put the `validator_addresses` into the same order as the
        // `self.validator_peer_id_cache` so that we can simultaneously iterate
        // over them.
        let mut sorted_validator_addresses: Vec<_> = validators
            .validators
            .iter()
            .map(|validator| &validator.address)
            .collect();
        sorted_validator_addresses.sort_unstable();
        let mut sorted_validator_addresses = sorted_validator_addresses.into_iter();
        let mut cur_key = sorted_validator_addresses.next();

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
                while cur_key.map(|k| k < key).unwrap_or(false) {
                    cur_key = sorted_validator_addresses.next();
                }
                Some(key) == cur_key
            });

        *self.validators.write() = Some(validators.clone());
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

    fn receive<M>(&self) -> MessageStream<M, N::PeerId>
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
                        // Check that each message actually comes from the peer that it
                        // claims it comes from. Reject it otherwise. Besides the peer ID we
                        // resolved, accept the peer with the validator's newest verified claim, so
                        // a validator that moved to another node is not ignored until our cache
                        // catches up.
                        if !self_.accept_sender(message.validator_id, peer_id) {
                            let validator_peer_id = self_.get_validator_cache(message.validator_id).potentially_outdated_peer_id();
                            warn!(%peer_id, ?validator_peer_id, claimed_validator_id = message.validator_id, "Dropping validator message");
                            return None;
                        }
                        Some((message.inner, message.validator_id, peer_id))
                    }
                }),
        )
    }

    fn receive_requests<TRequest: Request>(
        &self,
    ) -> BoxStream<'static, (TRequest, <Self::NetworkType as Network>::RequestId, u16)> {
        let self_ = self.arc_clone();

        self.network
            .receive_requests::<ValidatorMessage<TRequest>>()
            .filter_map(move |(message, request_id, peer_id)| {
                let self_ = self_.arc_clone();
                async move {
                    // Check that each request actually comes from the peer that it
                    // claims it comes from. Reject it otherwise. See `receive` above for why the
                    // peer contact book is consulted as well.
                    if !self_.accept_sender(message.validator_id, peer_id) {
                        let validator_peer_id = self_.get_validator_cache(message.validator_id).potentially_outdated_peer_id();
                        warn!(%peer_id, ?validator_peer_id, claimed_validator_id = message.validator_id, "Dropping validator request");
                        return None;
                    }
                    Some((message.inner, request_id, message.validator_id))
                }
            })
            .boxed()
    }

    async fn respond<TRequest: Request>(
        &self,
        request_id: <Self::NetworkType as Network>::RequestId,
        response: TRequest::Response,
    ) -> Result<(), Self::Error> {
        self.network
            .respond::<TRequest>(request_id, response)
            .await
            .map_err(Into::into)
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
        validator_address: &Address,
        signing_key_pair: &KeyPair,
    ) -> Result<(), Self::Error> {
        let peer_id = self.network.get_local_peer_id();
        let record = ValidatorRecord::new(
            peer_id,
            validator_address.clone(),
            (OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000) as u64,
        );
        self.network
            .dht_put(validator_address, &record, signing_key_pair)
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

    fn get_peer_id(&self, validator_id: u16) -> Option<<Self::NetworkType as Network>::PeerId> {
        self.get_validator_cache(validator_id)
            .potentially_outdated_peer_id()
    }

    fn set_validator_claim_signer(&self, signer: Option<ValidatorClaimSigner>) {
        self.network.set_validator_claim_signer(signer)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use futures::future;
    use nimiq_bls::CompressedPublicKey;
    use nimiq_keys::{Ed25519PublicKey, KeyPair, SecureGenerate};
    use nimiq_network_interface::request::{MessageMarker, RequestMarker};
    use nimiq_network_mock::{MockHub, MockNetwork, MockPeerId};
    use nimiq_test_log::test;
    use nimiq_test_utils::test_rng;

    use super::*;

    type ValNet = ValidatorNetworkImpl<MockNetwork>;

    fn validator_address(seed: u8) -> Address {
        let mut bytes = [0u8; 20];
        bytes[0] = seed;
        Address::from(bytes)
    }

    fn no_fallback() -> Arc<DhtFallback<MockNetwork>> {
        Arc::new(|_| Box::pin(future::ready(None)))
    }

    /// Makes `advertiser` claim `address` in the (mock) peer contact book.
    fn advertise(advertiser: &MockNetwork, address: &Address) {
        let key_pair = KeyPair::generate(&mut test_rng(false));
        advertiser
            .set_validator_claim_signer(Some(ValidatorClaimSigner::new(address.clone(), key_pair)));
    }

    /// Publishes a DHT record pointing `address` at `peer_id`.
    async fn publish_dht(network: &MockNetwork, address: &Address, peer_id: MockPeerId) {
        let key_pair = KeyPair::generate(&mut test_rng(false));
        let record = ValidatorRecord::new(peer_id, address.clone(), 1);
        network.dht_put(address, &record, &key_pair).await.unwrap();
    }

    fn fallback_to(peer_id: MockPeerId) -> Arc<DhtFallback<MockNetwork>> {
        Arc::new(move |_| Box::pin(future::ready(Some(peer_id))))
    }

    /// Like `fallback_to`, but also counts how often the fallback is asked.
    fn counting_fallback_to(
        peer_id: MockPeerId,
    ) -> (Arc<DhtFallback<MockNetwork>>, Arc<AtomicUsize>) {
        let asked = Arc::new(AtomicUsize::new(0));
        let fallback: Arc<DhtFallback<MockNetwork>> = {
            let asked = Arc::clone(&asked);
            Arc::new(move |_| {
                asked.fetch_add(1, Ordering::Relaxed);
                Box::pin(future::ready(Some(peer_id)))
            })
        };
        (fallback, asked)
    }

    const VALIDATOR_ID: u16 = 0;

    /// A validator network for `us` in which `address` is the only validator, at `VALIDATOR_ID`.
    fn validator_network(us: MockNetwork, address: &Address) -> ValNet {
        let network = ValNet::new(Arc::new(us));
        network.set_validators(&Validators::new(vec![Validator::new(
            address.clone(),
            CompressedPublicKey::default(),
            Ed25519PublicKey::default(),
            0..1,
        )]));
        network.set_validator_id(Some(VALIDATOR_ID + 1));
        network
    }

    fn cache_state(network: &ValNet, address: &Address) -> Option<CacheState<MockPeerId>> {
        network.validator_peer_id_cache.read().get(address).copied()
    }

    /// Lets a resolution of `address` that is in progress finish.
    async fn settle(network: &ValNet, address: &Address) {
        for _ in 0..100 {
            if !matches!(
                cache_state(network, address),
                Some(CacheState::InProgress(..))
            ) {
                return;
            }
            tokio::task::yield_now().await;
        }
        panic!("the resolution did not finish");
    }

    #[derive(Debug, Deserialize, Serialize)]
    struct Ping(u32);

    impl RequestCommon for Ping {
        type Kind = RequestMarker;
        type Response = u32;
        const TYPE_ID: u16 = 1;
        const MAX_REQUESTS: u32 = 10;
    }

    #[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
    struct Note(u32);

    impl RequestCommon for Note {
        type Kind = MessageMarker;
        type Response = ();
        const TYPE_ID: u16 = 2;
        const MAX_REQUESTS: u32 = 10;
    }

    /// Makes `peer` answer every `Ping` it receives over the validator network.
    fn answer_pings(peer: Arc<MockNetwork>) {
        let mut requests = peer.receive_requests::<ValidatorMessage<Ping>>();
        spawn(async move {
            while let Some((request, request_id, _)) = requests.next().await {
                let _ = peer
                    .respond::<ValidatorMessage<Ping>>(request_id, request.inner.0)
                    .await;
            }
        });
    }

    /// Makes a request to `validator`, the validator at `address`, fail once, as it would while
    /// the validator restarts, and checks that requests reach it again once it answers under the
    /// same peer ID.
    async fn assert_requests_recover(
        network: &ValNet,
        validator: Arc<MockNetwork>,
        address: &Address,
    ) {
        let validator_peer_id = validator.get_local_peer_id();
        network.get_validator_cache(VALIDATOR_ID);
        settle(network, address).await;
        assert!(matches!(
            cache_state(network, address),
            Some(CacheState::Resolved(peer_id)) if peer_id == validator_peer_id
        ));

        // The validator does not answer requests yet.
        assert!(matches!(
            network.request(Ping(1), VALIDATOR_ID).await,
            Err(NetworkError::Request(_))
        ));
        assert!(matches!(
            cache_state(network, address),
            Some(CacheState::Empty(peer_id)) if peer_id == validator_peer_id
        ));

        // Now it does. The next request finds the cache entry emptied and resolves it again.
        answer_pings(validator);
        assert!(matches!(
            network.request(Ping(2), VALIDATOR_ID).await,
            Err(NetworkError::Unreachable)
        ));
        settle(network, address).await;
        assert_eq!(network.request(Ping(3), VALIDATOR_ID).await.unwrap(), 3);
    }

    #[test(tokio::test)]
    async fn a_connected_contact_resolves_without_the_dht() {
        let mut hub = MockHub::default();
        let us = hub.new_network();
        let peer = hub.new_network();
        let stale = hub.new_network();
        us.dial_mock(&peer);

        let address = validator_address(1);
        advertise(&peer, &address);
        // The DHT points somewhere else; the connected contact must win.
        publish_dht(&us, &address, stale.get_local_peer_id()).await;

        let resolved = ValNet::resolve_peer_id(&us, &address, no_fallback(), None)
            .await
            .unwrap();

        assert_eq!(resolved, Some(peer.get_local_peer_id()));
    }

    #[test(tokio::test)]
    async fn an_unconnected_contact_does_not_beat_the_dht() {
        let mut hub = MockHub::default();
        let us = hub.new_network();
        let unconnected = hub.new_network();
        let published = hub.new_network();
        // `published` makes no validator claim; it is only here so that we have a connection,
        // which the mock network requires before it accepts a DHT put.
        us.dial_mock(&published);

        let address = validator_address(1);
        advertise(&unconnected, &address);
        publish_dht(&us, &address, published.get_local_peer_id()).await;

        let resolved = ValNet::resolve_peer_id(&us, &address, no_fallback(), None)
            .await
            .unwrap();

        assert_eq!(resolved, Some(published.get_local_peer_id()));
    }

    #[test(tokio::test)]
    async fn an_unconnected_contact_resolves_when_the_dht_is_empty() {
        let mut hub = MockHub::default();
        let us = hub.new_network();
        let unconnected = hub.new_network();

        let address = validator_address(1);
        advertise(&unconnected, &address);

        let resolved = ValNet::resolve_peer_id(&us, &address, no_fallback(), None)
            .await
            .unwrap();

        assert_eq!(resolved, Some(unconnected.get_local_peer_id()));
    }

    #[test(tokio::test)]
    async fn the_fallback_is_not_asked_when_the_dht_answers() {
        let mut hub = MockHub::default();
        let us = hub.new_network();
        let published = hub.new_network();
        let other = hub.new_network();
        us.dial_mock(&published);

        let address = validator_address(1);
        publish_dht(&us, &address, published.get_local_peer_id()).await;
        let (fallback, asked) = counting_fallback_to(other.get_local_peer_id());

        let resolved = ValNet::resolve_peer_id(&us, &address, fallback, None)
            .await
            .unwrap();

        assert_eq!(resolved, Some(published.get_local_peer_id()));
        assert_eq!(asked.load(Ordering::Relaxed), 0);
    }

    #[test(tokio::test)]
    async fn an_unconnected_contact_beats_the_fallback() {
        let mut hub = MockHub::default();
        let us = hub.new_network();
        let unconnected = hub.new_network();
        let listed = hub.new_network();

        let address = validator_address(1);
        advertise(&unconnected, &address);
        let (fallback, asked) = counting_fallback_to(listed.get_local_peer_id());

        let resolved = ValNet::resolve_peer_id(&us, &address, fallback, None)
            .await
            .unwrap();

        // A claim the validator signed wins over the unsigned fallback, which is not even asked.
        assert_eq!(resolved, Some(unconnected.get_local_peer_id()));
        assert_eq!(asked.load(Ordering::Relaxed), 0);
    }

    #[test(tokio::test)]
    async fn the_fallback_is_asked_when_no_signed_source_answers() {
        let mut hub = MockHub::default();
        let us = hub.new_network();
        let listed = hub.new_network();

        let address = validator_address(1);
        let (fallback, asked) = counting_fallback_to(listed.get_local_peer_id());

        let resolved = ValNet::resolve_peer_id(&us, &address, fallback, None)
            .await
            .unwrap();

        assert_eq!(resolved, Some(listed.get_local_peer_id()));
        assert_eq!(asked.load(Ordering::Relaxed), 1);
    }

    #[test(tokio::test)]
    async fn the_peer_that_just_failed_is_skipped() {
        let mut hub = MockHub::default();
        let us = hub.new_network();
        let failed = hub.new_network();
        let other = hub.new_network();
        us.dial_mock(&failed);
        us.dial_mock(&other);

        let address = validator_address(1);
        advertise(&failed, &address);
        advertise(&other, &address);

        let resolved = ValNet::resolve_peer_id(
            &us,
            &address,
            no_fallback(),
            Some(failed.get_local_peer_id()),
        )
        .await
        .unwrap();

        assert_eq!(resolved, Some(other.get_local_peer_id()));
    }

    #[test(tokio::test)]
    async fn a_stale_dht_record_does_not_hide_a_validator_that_moved() {
        let mut hub = MockHub::default();
        let us = hub.new_network();
        let relay = hub.new_network();
        let failed = hub.new_network();
        let moved_to = hub.new_network();
        // `relay` is only here so that we have a connection, which the mock network requires
        // before it accepts a DHT put.
        us.dial_mock(&relay);

        let address = validator_address(1);
        advertise(&moved_to, &address);
        publish_dht(&us, &address, failed.get_local_peer_id()).await;

        let resolved = ValNet::resolve_peer_id(
            &us,
            &address,
            no_fallback(),
            Some(failed.get_local_peer_id()),
        )
        .await
        .unwrap();

        assert_eq!(resolved, Some(moved_to.get_local_peer_id()));
    }

    #[test(tokio::test)]
    async fn a_stale_fallback_does_not_hide_a_validator_that_moved() {
        let mut hub = MockHub::default();
        let us = hub.new_network();
        let failed = hub.new_network();
        let moved_to = hub.new_network();

        let address = validator_address(1);
        advertise(&moved_to, &address);

        let resolved = ValNet::resolve_peer_id(
            &us,
            &address,
            fallback_to(failed.get_local_peer_id()),
            Some(failed.get_local_peer_id()),
        )
        .await
        .unwrap();

        assert_eq!(resolved, Some(moved_to.get_local_peer_id()));
    }

    #[test(tokio::test)]
    async fn the_peer_that_just_failed_is_resolved_again_from_the_dht() {
        let mut hub = MockHub::default();
        let us = hub.new_network();
        let failed = hub.new_network();
        us.dial_mock(&failed);

        let address = validator_address(1);
        publish_dht(&us, &address, failed.get_local_peer_id()).await;

        let resolved = ValNet::resolve_peer_id(
            &us,
            &address,
            no_fallback(),
            Some(failed.get_local_peer_id()),
        )
        .await
        .unwrap();

        assert_eq!(resolved, Some(failed.get_local_peer_id()));
    }

    #[test(tokio::test)]
    async fn the_peer_that_just_failed_is_resolved_again_from_the_fallback() {
        let mut hub = MockHub::default();
        let us = hub.new_network();
        let failed = hub.new_network();

        let address = validator_address(1);

        let resolved = ValNet::resolve_peer_id(
            &us,
            &address,
            fallback_to(failed.get_local_peer_id()),
            Some(failed.get_local_peer_id()),
        )
        .await
        .unwrap();

        assert_eq!(resolved, Some(failed.get_local_peer_id()));
    }

    #[test(tokio::test)]
    async fn the_peer_that_just_failed_is_resolved_again_from_its_contact() {
        let mut hub = MockHub::default();
        let us = hub.new_network();
        let failed = hub.new_network();

        let address = validator_address(1);
        advertise(&failed, &address);

        let resolved = ValNet::resolve_peer_id(
            &us,
            &address,
            no_fallback(),
            Some(failed.get_local_peer_id()),
        )
        .await
        .unwrap();

        assert_eq!(resolved, Some(failed.get_local_peer_id()));
    }

    #[test(tokio::test)]
    async fn the_fallback_does_not_override_the_dht_after_a_failure() {
        let mut hub = MockHub::default();
        let us = hub.new_network();
        let failed = hub.new_network();
        let other = hub.new_network();
        us.dial_mock(&failed);

        let address = validator_address(1);
        publish_dht(&us, &address, failed.get_local_peer_id()).await;

        let (fallback, asked) = counting_fallback_to(other.get_local_peer_id());

        let resolved =
            ValNet::resolve_peer_id(&us, &address, fallback, Some(failed.get_local_peer_id()))
                .await
                .unwrap();

        assert_eq!(resolved, Some(failed.get_local_peer_id()));
        assert_eq!(asked.load(Ordering::Relaxed), 0);
    }

    #[test(tokio::test)]
    async fn a_peer_that_nothing_points_to_is_not_resolved_again() {
        let mut hub = MockHub::default();
        let us = hub.new_network();
        let failed = hub.new_network();
        us.dial_mock(&failed);

        let address = validator_address(1);

        let resolved = ValNet::resolve_peer_id(
            &us,
            &address,
            no_fallback(),
            Some(failed.get_local_peer_id()),
        )
        .await
        .unwrap();

        assert_eq!(resolved, None);
    }

    #[test(tokio::test)]
    async fn requests_recover_from_a_transient_failure() {
        let mut hub = MockHub::default();
        let us = hub.new_network();
        let validator = Arc::new(hub.new_network());
        us.dial_mock(&validator);

        let address = validator_address(1);
        publish_dht(&us, &address, validator.get_local_peer_id()).await;

        let network = validator_network(us, &address);
        assert_requests_recover(&network, validator, &address).await;
    }

    #[test(tokio::test)]
    async fn requests_recover_from_a_transient_failure_without_the_dht() {
        let mut hub = MockHub::default();
        let us = hub.new_network();
        let validator = Arc::new(hub.new_network());
        us.dial_mock(&validator);

        // Only the peer contact book knows the validator.
        let address = validator_address(1);
        advertise(&validator, &address);

        let network = validator_network(us, &address);
        assert_requests_recover(&network, validator, &address).await;
    }

    #[test(tokio::test)]
    async fn requests_recover_after_resolving_the_failed_peer_failed_too() {
        let mut hub = MockHub::default();
        let us = hub.new_network();
        let validator = Arc::new(hub.new_network());
        us.dial_mock(&validator);
        let validator_peer_id = validator.get_local_peer_id();

        let address = validator_address(1);
        advertise(&validator, &address);
        let network = validator_network(us, &address);
        network.get_validator_cache(VALIDATOR_ID);
        settle(&network, &address).await;

        // A request to the validator fails, and so does resolving it again, here because its
        // claim is gone for a moment.
        assert!(matches!(
            network.request(Ping(1), VALIDATOR_ID).await,
            Err(NetworkError::Request(_))
        ));
        validator.set_validator_claim_signer(None);
        assert!(matches!(
            network.request(Ping(2), VALIDATOR_ID).await,
            Err(NetworkError::Unreachable)
        ));
        settle(&network, &address).await;
        assert!(matches!(
            cache_state(&network, &address),
            Some(CacheState::Error(Some(peer_id))) if peer_id == validator_peer_id
        ));

        // It is back under the same peer ID. Retrying from the error still passes it over first,
        // but as nothing points elsewhere, requests reach it again.
        advertise(&validator, &address);
        answer_pings(validator);
        assert!(matches!(
            network.request(Ping(3), VALIDATOR_ID).await,
            Err(NetworkError::Unreachable)
        ));
        settle(&network, &address).await;
        assert_eq!(network.request(Ping(4), VALIDATOR_ID).await.unwrap(), 4);
    }

    /// A validator network for `us`, in which the cache entry of the validator at `address` points
    /// to `cached`. `claimant` then advertises a verified claim to be that validator, while
    /// `stranger` makes no claim. All of them are connected to us.
    async fn validator_network_with_claimant(
        us: MockNetwork,
        address: &Address,
        cached: &MockNetwork,
        claimant: &MockNetwork,
        stranger: &MockNetwork,
    ) -> ValNet {
        for peer in [cached, claimant, stranger] {
            us.dial_mock(peer);
        }
        publish_dht(&us, address, cached.get_local_peer_id()).await;
        let network = validator_network(us, address);
        network.get_validator_cache(VALIDATOR_ID);
        settle(&network, address).await;
        assert!(matches!(
            cache_state(&network, address),
            Some(CacheState::Resolved(peer_id)) if peer_id == cached.get_local_peer_id()
        ));
        advertise(claimant, address);
        network
    }

    /// Sends `Note(note)` to `to` over the validator network, claiming to be the validator at
    /// `VALIDATOR_ID`.
    async fn send_note(sender: &MockNetwork, to: MockPeerId, note: u32) {
        let message = ValidatorMessage {
            validator_id: VALIDATOR_ID,
            inner: Note(note),
        };
        sender.message(message, to).await.unwrap();
    }

    #[test(tokio::test)]
    async fn messages_are_only_accepted_from_the_cached_peer_or_the_newest_claimant() {
        let mut hub = MockHub::default();
        let us = hub.new_network();
        let cached = hub.new_network();
        let claimant = hub.new_network();
        let stranger = hub.new_network();
        let us_peer_id = us.get_local_peer_id();

        let address = validator_address(1);
        let network =
            validator_network_with_claimant(us, &address, &cached, &claimant, &stranger).await;
        let mut messages = network.receive::<Note>();

        for (sender, note) in [(&stranger, 1), (&claimant, 2), (&cached, 3)] {
            send_note(sender, us_peer_id, note).await;
        }

        // The stranger's message is dropped. The mock delivers messages right away, so the ones
        // accepted are ready without waiting.
        assert_eq!(
            messages.next().now_or_never(),
            Some(Some((Note(2), VALIDATOR_ID, claimant.get_local_peer_id())))
        );
        assert_eq!(
            messages.next().now_or_never(),
            Some(Some((Note(3), VALIDATOR_ID, cached.get_local_peer_id())))
        );
        assert!(messages.next().now_or_never().is_none());

        // Hearing the claimant does not change where we send to.
        assert_eq!(
            network.get_peer_id(VALIDATOR_ID),
            Some(cached.get_local_peer_id())
        );
    }

    #[test(tokio::test)]
    async fn requests_are_only_accepted_from_the_cached_peer_or_the_newest_claimant() {
        let mut hub = MockHub::default();
        let us = hub.new_network();
        let cached = hub.new_network();
        let claimant = hub.new_network();
        let stranger = hub.new_network();
        let us_peer_id = us.get_local_peer_id();

        let address = validator_address(1);
        let network =
            validator_network_with_claimant(us, &address, &cached, &claimant, &stranger).await;
        let mut requests = network.receive_requests::<Ping>();

        // The stranger's request arrives first. It is never answered, so only poll it once.
        let stranger_request = stranger.request(
            ValidatorMessage {
                validator_id: VALIDATOR_ID,
                inner: Ping(1),
            },
            us_peer_id,
        );
        let mut stranger_request = std::pin::pin!(stranger_request);
        assert!(stranger_request.as_mut().now_or_never().is_none());

        for (sender, ping) in [(&claimant, 2), (&cached, 3)] {
            let request = sender.request(
                ValidatorMessage {
                    validator_id: VALIDATOR_ID,
                    inner: Ping(ping),
                },
                us_peer_id,
            );
            let mut request = std::pin::pin!(request);
            assert!(request.as_mut().now_or_never().is_none());

            // The stranger's request was dropped, so this is the one we just sent.
            let (incoming, request_id, validator_id) = requests
                .next()
                .now_or_never()
                .flatten()
                .expect("the request was dropped");
            assert_eq!((incoming.0, validator_id), (ping, VALIDATOR_ID));
            network
                .respond::<Ping>(request_id, incoming.0)
                .await
                .unwrap();
            assert_eq!(request.await.unwrap(), ping);
        }
    }

    #[test(tokio::test)]
    async fn only_the_newest_claimant_is_heard() {
        let mut hub = MockHub::default();
        let us = hub.new_network();
        let older = hub.new_network();
        let newest = hub.new_network();
        let us_peer_id = us.get_local_peer_id();

        let address = validator_address(1);
        us.dial_mock(&older);
        us.dial_mock(&newest);
        advertise(&older, &address);
        advertise(&newest, &address);
        let network = validator_network(us, &address);
        let mut messages = network.receive::<Note>();

        send_note(&older, us_peer_id, 1).await;
        send_note(&newest, us_peer_id, 2).await;
        settle(&network, &address).await;

        assert_eq!(
            messages.next().now_or_never(),
            Some(Some((Note(2), VALIDATOR_ID, newest.get_local_peer_id())))
        );
        assert!(messages.next().now_or_never().is_none());
    }

    #[test(tokio::test)]
    async fn only_the_newest_unconnected_claim_is_tried() {
        let mut hub = MockHub::default();
        let us = hub.new_network();
        let left_behind = hub.new_network();
        let newest = hub.new_network();
        let listed = hub.new_network();

        let address = validator_address(1);
        advertise(&left_behind, &address);
        advertise(&newest, &address);

        // The newest claim is tried first...
        let resolved = ValNet::resolve_peer_id(&us, &address, no_fallback(), None)
            .await
            .unwrap();
        assert_eq!(resolved, Some(newest.get_local_peer_id()));

        // ...and once it failed, the fallback is asked rather than a node the validator left
        // behind, which could otherwise alternate with the newest one for as long as both live.
        let (fallback, asked) = counting_fallback_to(listed.get_local_peer_id());
        let resolved =
            ValNet::resolve_peer_id(&us, &address, fallback, Some(newest.get_local_peer_id()))
                .await
                .unwrap();
        assert_eq!(resolved, Some(listed.get_local_peer_id()));
        assert_eq!(asked.load(Ordering::Relaxed), 1);
    }

    // A node the validator left behind may still be connected. It must not take precedence over
    // the validator's newest claim, nor over its DHT record: its messages would not even be heard.
    #[test(tokio::test)]
    async fn a_connected_node_the_validator_left_behind_is_not_used() {
        let mut hub = MockHub::default();
        let us = hub.new_network();
        let left_behind = hub.new_network();
        let newest = hub.new_network();
        let recorded = hub.new_network();

        let address = validator_address(1);
        us.dial_mock(&left_behind);
        advertise(&left_behind, &address);
        advertise(&newest, &address);

        let resolved = ValNet::resolve_peer_id(&us, &address, no_fallback(), None)
            .await
            .unwrap();
        assert_eq!(resolved, Some(newest.get_local_peer_id()));

        publish_dht(&us, &address, recorded.get_local_peer_id()).await;
        let resolved = ValNet::resolve_peer_id(&us, &address, no_fallback(), None)
            .await
            .unwrap();
        assert_eq!(resolved, Some(recorded.get_local_peer_id()));
    }

    #[test(tokio::test)]
    async fn a_claim_to_the_peer_that_just_failed_does_not_keep_the_fallback_from_being_asked() {
        let mut hub = MockHub::default();
        let us = hub.new_network();
        let failed = hub.new_network();
        let listed = hub.new_network();

        let address = validator_address(1);
        advertise(&failed, &address);
        let (fallback, asked) = counting_fallback_to(listed.get_local_peer_id());

        let resolved =
            ValNet::resolve_peer_id(&us, &address, fallback, Some(failed.get_local_peer_id()))
                .await
                .unwrap();

        assert_eq!(resolved, Some(listed.get_local_peer_id()));
        assert_eq!(asked.load(Ordering::Relaxed), 1);
    }

    #[test(tokio::test)]
    async fn we_never_resolve_to_ourselves() {
        let mut hub = MockHub::default();
        let us = hub.new_network();

        let address = validator_address(1);
        advertise(&us, &address);

        assert!(us.get_validator_peer_ids(&address).is_empty());
    }

    #[test(tokio::test)]
    async fn dropping_the_signer_stops_the_advertisement() {
        let mut hub = MockHub::default();
        let us = hub.new_network();
        let peer = hub.new_network();

        let address = validator_address(1);
        advertise(&peer, &address);
        assert_eq!(us.get_validator_peer_ids(&address).len(), 1);

        peer.set_validator_claim_signer(None);
        assert!(us.get_validator_peer_ids(&address).is_empty());
    }
}
