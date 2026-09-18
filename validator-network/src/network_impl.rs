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
use crate::validator_record::{ValidatorRecord, ValidatorRecordSigner};

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
    /// Sources are tried cheapest first. A peer we are already connected to whose gossiped
    /// contact carries a verified claim for this validator answers immediately, while a DHT
    /// lookup can take seconds or time out entirely. A contact for a peer we are *not* connected
    /// to is only consulted after the DHT and the HTTPS fallback, so that a contact left behind
    /// by a validator that moved can never win over the record the validator published itself.
    ///
    /// `exclude` is the peer ID we used for this validator before, when a request to it failed.
    async fn resolve_peer_id(
        network: &N,
        validator_address: &Address,
        fallback: Arc<DhtFallback<N>>,
        exclude: Option<N::PeerId>,
    ) -> Result<Option<N::PeerId>, NetworkError<N::Error>> {
        if let Some(peer_id) =
            Self::resolve_peer_id_contact_book(network, validator_address, exclude, true)
        {
            log::debug!(%peer_id, %validator_address, "Resolved validator peer ID from a connected peer contact");
            return Ok(Some(peer_id));
        }

        let result = Self::resolve_peer_id_dht(network, validator_address).await;
        if let Ok(Some(peer_id)) = result {
            log::debug!(%peer_id, %validator_address, "Resolved validator peer ID from the DHT");
            return result;
        }

        if let Some(peer_id) = fallback(validator_address.clone()).await {
            log::debug!(%peer_id, %validator_address, "Resolved validator peer ID from the DHT fallback");
            return Ok(Some(peer_id));
        }

        if let Some(peer_id) =
            Self::resolve_peer_id_contact_book(network, validator_address, exclude, false)
        {
            log::debug!(%peer_id, %validator_address, "Resolved validator peer ID from a peer contact");
            return Ok(Some(peer_id));
        }

        result
    }

    /// Looks up the peer ID for a validator address in the peer contact book.
    ///
    /// Only contacts whose validator claim this node verified are considered. With
    /// `connected_only`, the peer must already be connected, which makes the answer both instant
    /// and known to be reachable.
    fn resolve_peer_id_contact_book(
        network: &N,
        validator_address: &Address,
        exclude: Option<N::PeerId>,
        connected_only: bool,
    ) -> Option<N::PeerId> {
        network
            .get_validator_peer_ids(validator_address)
            .into_iter()
            .filter(|peer_id| Some(*peer_id) != exclude)
            .find(|peer_id| !connected_only || network.has_peer(*peer_id))
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

    /// Looks up the peer ID for a validator address in the DHT and updates
    /// the internal cache.
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
        // Do not resolve back to the peer ID we just failed to reach.
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

    /// Whether `peer_id` is known to belong to `validator_id` through a verified peer contact.
    fn is_verified_validator_peer(&self, validator_id: u16, peer_id: &N::PeerId) -> bool {
        let Some(validator_address) = self.validator_address(validator_id) else {
            return false;
        };
        // Note that the validators lock is released before we ask the network, so that the
        // contact book lock is never taken while holding it.
        self.network
            .get_validator_peer_ids(&validator_address)
            .contains(peer_id)
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
                        let cached_peer_id = self_.get_validator_cache(message.validator_id).potentially_outdated_peer_id();
                        // Check that each message actually comes from the peer that it
                        // claims it comes from. Reject it otherwise. Besides the peer ID we
                        // resolved, accept any peer whose gossiped contact carries a verified
                        // claim for this validator, so a validator that moved to another node is
                        // not ignored until our cache catches up.
                        if cached_peer_id != Some(peer_id)
                            && !self_.is_verified_validator_peer(message.validator_id, &peer_id)
                        {
                            warn!(%peer_id, ?cached_peer_id, claimed_validator_id = message.validator_id, "Dropping validator message");
                            return None;
                        }
                        Some((message.inner, message.validator_id))
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
                    let cached_peer_id = self_.get_validator_cache(message.validator_id).potentially_outdated_peer_id();
                    // Check that each request actually comes from the peer that it
                    // claims it comes from. Reject it otherwise. See `receive` above for why the
                    // peer contact book is consulted as well.
                    if cached_peer_id != Some(peer_id)
                        && !self_.is_verified_validator_peer(message.validator_id, &peer_id)
                    {
                        warn!(%peer_id, ?cached_peer_id, claimed_validator_id = message.validator_id, "Dropping validator request");
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

    fn set_validator_record_signer(&self, signer: Option<ValidatorRecordSigner>) {
        self.network.set_validator_record_signer(signer)
    }
}

#[cfg(test)]
mod tests {
    use futures::future;
    use nimiq_keys::{KeyPair, SecureGenerate};
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
        advertiser.set_validator_record_signer(Some(ValidatorRecordSigner::new(
            address.clone(),
            key_pair,
        )));
    }

    /// Publishes a DHT record pointing `address` at `peer_id`.
    async fn publish_dht(network: &MockNetwork, address: &Address, peer_id: MockPeerId) {
        let key_pair = KeyPair::generate(&mut test_rng(false));
        let record = ValidatorRecord::new(peer_id, address.clone(), 1);
        network.dht_put(address, &record, &key_pair).await.unwrap();
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

        peer.set_validator_record_signer(None);
        assert!(us.get_validator_peer_ids(&address).is_empty());
    }
}
