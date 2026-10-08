use std::{
    collections::BTreeMap,
    error::Error,
    fmt::Debug,
    future,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
    time::Duration,
};

use futures::{
    future::{join_all, BoxFuture, Shared},
    stream::BoxStream,
    FutureExt, StreamExt, TryFutureExt,
};
use log::warn;
use nimiq_keys::{Address, KeyPair};
use nimiq_network_interface::{
    network::{CloseReason, MsgAcceptance, Network, NetworkEvent, SubscribeEvents, Topic},
    peer_info::Services,
    request::{InboundRequestError, Message, Request, RequestCommon, RequestError},
};
use nimiq_primitives::slots_allocation::{Validator, Validators};
use nimiq_serde::{Deserialize, Serialize};
use nimiq_time::timeout;
use nimiq_utils::{
    spawn,
    tagged_signing::{TaggedKeyPair, TaggedSigned},
};
use parking_lot::{Mutex, RwLock};
use time::OffsetDateTime;

use super::{MessageStream, NetworkError, PubsubId, ValidatorNetwork};
use crate::{
    binding::{Binding, CacheState},
    record::{verify_record, RecordAnnouncement, RecordsRequest, SignedValidatorRecord},
    validator_record::ValidatorRecord,
};

/// Milliseconds since the Unix epoch.
fn now_ms() -> u64 {
    (OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000) as u64
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
    /// The peer ID each validator is bound to, by validator address.
    validator_peer_id_cache: Arc<RwLock<BTreeMap<Address, CacheState<N::PeerId>>>>,
    dht_fallback: Arc<DhtFallback<N>>,
    /// Our own signed record, once `set_public_key` created it.
    own_record: Arc<RwLock<Option<SignedValidatorRecord<N::PeerId>>>>,
    /// The timestamp of the newest verified copy of our own record seen at or past ours, zero if
    /// none. See [`ValidatorNetwork::own_record_superseded_at`].
    superseded_at: Arc<AtomicU64>,
    /// The newest verified record of each active validator that we received signed, to answer
    /// pulls with and to fall back on.
    known_records: Arc<RwLock<BTreeMap<Address, SignedValidatorRecord<N::PeerId>>>>,
    /// The pull round in flight, shared by every resolution that starts meanwhile.
    pull: Arc<Mutex<Option<Shared<BoxFuture<'static, ()>>>>>,
    /// Whether the tasks receiving records from peers and answering pulls run.
    receiving: Arc<AtomicBool>,
    /// Whether the task announcing our record to joining validator peers runs.
    announcing_on_join: Arc<AtomicBool>,
}

impl<N> ValidatorNetworkImpl<N>
where
    N: Network,
    N::PeerId: Serialize + Deserialize,
    N::Error: Sync + Send,
{
    /// How long a pull of records from a peer may take before resolution moves on to the DHT.
    pub const PULL_TIMEOUT: Duration = Duration::from_secs(2);
    /// How many connected validator peers one pull asks.
    pub const MAX_PULL_PEERS: usize = 3;

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
            own_record: Arc::new(RwLock::new(None)),
            superseded_at: Arc::new(AtomicU64::new(0)),
            known_records: Arc::new(RwLock::new(BTreeMap::new())),
            pull: Arc::new(Mutex::new(None)),
            receiving: Arc::new(AtomicBool::new(false)),
            announcing_on_join: Arc::new(AtomicBool::new(false)),
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
            own_record: Arc::clone(&self.own_record),
            superseded_at: Arc::clone(&self.superseded_at),
            known_records: Arc::clone(&self.known_records),
            pull: Arc::clone(&self.pull),
            receiving: Arc::clone(&self.receiving),
            announcing_on_join: Arc::clone(&self.announcing_on_join),
        }
    }

    /// Whether `peer_id` is a connected peer advertising the validator service.
    fn is_validator_peer(&self, peer_id: N::PeerId) -> bool {
        self.network
            .get_peer_info(peer_id)
            .is_some_and(|info| info.get_services().contains(Services::VALIDATOR))
    }

    /// Our record for `validator_address` under our current peer ID. It is created if we have
    /// none, or re-signed past a newer copy of it that was seen.
    fn own_record(
        &self,
        validator_address: &Address,
        signing_key_pair: &KeyPair,
    ) -> SignedValidatorRecord<N::PeerId> {
        let peer_id = self.network.get_local_peer_id();
        let superseded_at = self.superseded_at.load(Ordering::SeqCst);
        let mut own_record = self.own_record.write();
        if let Some(record) = own_record.as_ref()
            && record.record.validator_address == *validator_address
            && record.record.peer_id == peer_id
            && record.record.timestamp > superseded_at
        {
            return record.clone();
        }

        let timestamp = now_ms().max(superseded_at + 1);
        let record = ValidatorRecord::new(peer_id, validator_address.clone(), timestamp);
        let signature = signing_key_pair.tagged_sign(&record);
        let signed = TaggedSigned::new(record, signature);
        *own_record = Some(signed.clone());
        // A copy reported meanwhile may be newer still; that one stays reported.
        let _ = self
            .superseded_at
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |seen| {
                (seen < timestamp).then_some(0)
            });
        signed
    }

    /// Sends `record`, our own, to `peer_id`. Failures are expected from peers without a
    /// receiver, e.g. old nodes, and only logged.
    async fn announce_to(&self, peer_id: N::PeerId, record: SignedValidatorRecord<N::PeerId>) {
        if let Err(error) = self
            .network
            .message(RecordAnnouncement { record }, peer_id)
            .await
        {
            log::debug!(%peer_id, ?error, "Could not announce our validator record");
        }
    }

    /// Sends `record`, our own, to every connected peer advertising the validator service.
    async fn announce_to_validator_peers(&self, record: &SignedValidatorRecord<N::PeerId>) {
        let announcements = self
            .network
            .get_peers()
            .into_iter()
            .filter(|peer_id| self.is_validator_peer(*peer_id))
            .map(|peer_id| self.announce_to(peer_id, record.clone()));
        join_all(announcements).await;
    }

    /// Starts announcing our record to validator peers as they join, once.
    fn start_announcing_on_join(&self) {
        if self.announcing_on_join.swap(true, Ordering::SeqCst) {
            return;
        }
        let self_ = self.arc_clone();
        let mut events = self.network.subscribe_events();
        spawn(async move {
            while let Some(event) = events.next().await {
                let Ok(NetworkEvent::PeerJoined(peer_id, info)) = event else {
                    continue;
                };
                if !info.get_services().contains(Services::VALIDATOR) {
                    continue;
                }
                let record = self_.own_record.read().clone();
                if let Some(record) = record {
                    self_.announce_to(peer_id, record).await;
                }
            }
        });
    }

    /// Starts receiving records announced by peers and answering pulls, once.
    fn start_receiving(&self) {
        if self.receiving.swap(true, Ordering::SeqCst) {
            return;
        }
        let self_ = self.arc_clone();
        let mut announcements = self
            .network
            .receive_messages::<RecordAnnouncement<N::PeerId>>();
        spawn(async move {
            while let Some((announcement, peer_id)) = announcements.next().await {
                self_.accept_record(announcement.record, Some(peer_id));
            }
        });

        let self_ = self.arc_clone();
        let mut pulls = self.network.receive_requests::<RecordsRequest<N::PeerId>>();
        spawn(async move {
            while let Some((_, request_id, peer_id)) = pulls.next().await {
                let records = self_.records_to_share();
                if let Err(error) = self_
                    .network
                    .respond::<RecordsRequest<N::PeerId>>(request_id, records)
                    .await
                {
                    log::debug!(%peer_id, ?error, "Could not answer a pull of validator records");
                }
            }
        });
    }

    /// The records we hold, ours last, so that a receiver that trims to its active set drops ours
    /// first if we are not in it.
    fn records_to_share(&self) -> Vec<SignedValidatorRecord<N::PeerId>> {
        let mut records: Vec<_> = self.known_records.read().values().cloned().collect();
        let own_record = self.own_record.read().clone();
        records.extend(own_record);
        records
    }

    /// Verifies a record against the active set and offers its binding to the validator's slot.
    /// `sender` is the peer that announced the record as its own, `None` if it was relayed.
    fn accept_record(
        &self,
        record: SignedValidatorRecord<N::PeerId>,
        sender: Option<N::PeerId>,
    ) -> Option<u16> {
        let validators = self.validators.read();
        let validator_id = match verify_record(&record, validators.as_ref()?, sender, now_ms()) {
            Ok(validator_id) => validator_id,
            Err(error) => {
                log::debug!(?sender, ?error, "Rejecting validator record");
                return None;
            }
        };

        // A copy of our own record never binds us elsewhere. One at least as new as ours means a
        // previous incarnation of ours signed later than we do now. It is reported, and the
        // validator re-signs past it through `set_public_key` (see `own_record`).
        let own_record = self.own_record.read().clone();
        if let Some(own_record) = own_record
            && own_record.record.validator_address == record.record.validator_address
        {
            if record.record.timestamp >= own_record.record.timestamp {
                log::info!(
                    timestamp = record.record.timestamp,
                    "A copy of our validator record signed later than ours was seen"
                );
                self.superseded_at
                    .fetch_max(record.record.timestamp, Ordering::SeqCst);
            }
            return Some(validator_id);
        }

        let binding = Binding::signed(record.record.peer_id, record.record.timestamp);
        let mut cache = self.validator_peer_id_cache.write();
        let slot = cache
            .entry(record.record.validator_address.clone())
            .or_insert(CacheState::Error(None));
        if slot.offer(binding) {
            log::debug!(validator_id, peer_id = %binding.peer_id, "Bound validator to the peer of its record");
        }
        drop(cache);

        let mut known_records = self.known_records.write();
        let newer = known_records
            .get(&record.record.validator_address)
            .is_none_or(|known| known.record.timestamp < record.record.timestamp);
        if newer {
            known_records.insert(record.record.validator_address.clone(), record);
        }
        Some(validator_id)
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

    /// Whether the slot of `validator_address` is bound.
    fn is_bound(&self, validator_address: &Address) -> bool {
        self.validator_peer_id_cache
            .read()
            .get(validator_address)
            .is_some_and(|slot| slot.current().is_some())
    }

    /// The binding of the record we hold for `validator_address`, if any.
    fn held_binding(&self, validator_address: &Address) -> Option<Binding<N::PeerId>> {
        self.known_records
            .read()
            .get(validator_address)
            .map(|record| Binding::signed(record.record.peer_id, record.record.timestamp))
    }

    /// Joins the pull round in flight or starts one.
    async fn pull(&self) {
        let round = {
            let mut pull = self.pull.lock();
            match pull.as_ref() {
                Some(round) => round.clone(),
                None => {
                    let self_ = self.arc_clone();
                    let round = async move { self_.pull_round().await }.boxed().shared();
                    *pull = Some(round.clone());
                    round
                }
            }
        };
        round.await;
    }

    /// Asks a few connected validator peers for the records they hold and applies every record
    /// that verifies to the slot of its validator.
    async fn pull_round(&self) {
        let peers: Vec<_> = self
            .network
            .get_peers()
            .into_iter()
            .filter(|peer_id| self.is_validator_peer(*peer_id))
            .take(Self::MAX_PULL_PEERS)
            .collect();
        let requests = peers.into_iter().map(|peer_id| async move {
            let request = self
                .network
                .request(RecordsRequest::<N::PeerId>::default(), peer_id);
            match timeout(Self::PULL_TIMEOUT, request).await {
                Ok(Ok(records)) => self.accept_pulled(records),
                Ok(Err(error)) => log::debug!(%peer_id, ?error, "Pulling validator records failed"),
                Err(_) => log::debug!(%peer_id, "Pulling validator records timed out"),
            }
        });
        join_all(requests).await;
        *self.pull.lock() = None;
    }

    /// Applies pulled records, at most one per active validator.
    fn accept_pulled(&self, records: Vec<SignedValidatorRecord<N::PeerId>>) {
        let limit = self
            .validators
            .read()
            .as_ref()
            .map_or(0, |validators| validators.num_validators());
        for record in records.into_iter().take(limit) {
            self.accept_record(record, None);
        }
    }

    async fn resolve_peer_id_dht(
        network: &N,
        validator_address: &Address,
    ) -> Result<Option<Binding<N::PeerId>>, NetworkError<N::Error>> {
        Ok(network
            .dht_get::<_, ValidatorRecord<N::PeerId>, KeyPair>(validator_address)
            .await?
            .map(|record| Binding::signed(record.peer_id, record.timestamp)))
    }

    /// Resolves the binding for a validator address and applies the outcome to its slot.
    ///
    /// Peers are asked first, since they answer within milliseconds and their records bind the
    /// slot right away. The DHT is asked regardless, as its record may be newer. The unsigned
    /// fallback is asked only if neither bound the slot, and the record we hold is the last
    /// resort, so that a peer that failed is tried again only if nothing points elsewhere.
    ///
    /// The given `validator_id` is used for logging purposes only.
    async fn update_peer_id_cache(&self, validator_id: u16, validator_address: &Address) {
        self.pull().await;

        let mut outcome = match Self::resolve_peer_id_dht(&self.network, validator_address).await {
            Ok(binding) => binding,
            Err(error) => {
                log::debug!(validator_id, ?error, %validator_address, "Unable to resolve validator peer ID: Network error");
                None
            }
        };
        if outcome.is_none() && !self.is_bound(validator_address) {
            outcome = (self.dht_fallback)(validator_address.clone())
                .await
                .map(Binding::unsigned)
                .or_else(|| self.held_binding(validator_address));
        }
        match outcome {
            Some(binding) => {
                log::trace!(peer_id = %binding.peer_id, validator_id, %validator_address, "Resolved validator peer ID")
            }
            None => {
                log::debug!(validator_id, %validator_address, "Unable to resolve validator peer ID")
            }
        }

        // The slot may be gone if the validator left the active set meanwhile.
        if let Some(slot) = self
            .validator_peer_id_cache
            .write()
            .get_mut(validator_address)
        {
            slot.resolved(outcome);
        }
    }

    /// Returns the slot of a validator ID, starting a resolution if it has no usable binding.
    fn get_validator_cache(&self, validator_id: u16) -> CacheState<N::PeerId> {
        let validators = self.validators.read();
        let Some(validator) = Self::get_validator(validators.as_ref(), validator_id) else {
            return CacheState::Error(None);
        };

        // Fast path: nothing to start.
        if let Some(slot) = self.validator_peer_id_cache.read().get(&validator.address)
            && matches!(slot, CacheState::Resolved(_) | CacheState::InProgress(_))
        {
            return *slot;
        }

        let state = {
            let mut cache = self.validator_peer_id_cache.write();
            let slot = cache
                .entry(validator.address.clone())
                .or_insert(CacheState::Error(None));
            if !slot.start_resolving() {
                return *slot;
            }
            *slot
        };
        log::debug!(validator_id, validator_address = %validator.address, "Resolving validator peer ID");

        let self_ = self.arc_clone();
        let validator_address = validator.address.clone();
        spawn(async move {
            Self::update_peer_id_cache(&self_, validator_id, &validator_address).await;
        });
        state
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

        // Drop the binding only if the error happened for the peer we are bound to.
        if let Some(slot) = self
            .validator_peer_id_cache
            .write()
            .get_mut(&validator.address)
        {
            slot.mark_failed(*peer_id);
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

        self.known_records
            .write()
            .retain(|address, _| validators.get_validator_by_address(address).is_some());
        *self.validators.write() = Some(validators.clone());

        self.start_receiving();
        // Validators that entered the set could not verify our record before.
        let record = self.own_record.read().clone();
        if let Some(record) = record {
            let self_ = self.arc_clone();
            spawn(async move { self_.announce_to_validator_peers(&record).await });
        }
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
            .latest()
            .map(|binding| binding.peer_id)
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
        if let Some(peer_id) = self
            .get_validator_cache(validator_id)
            .current()
            .map(|binding| binding.peer_id)
        {
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
                        let validator_peer_id = self_.get_validator_cache(message.validator_id).latest().map(|binding| binding.peer_id);
                        // Check that each message actually comes from the peer that it
                        // claims it comes from. Reject it otherwise.
                        if validator_peer_id
                            .as_ref()
                            .map(|pid| *pid != peer_id)
                            .unwrap_or(true)
                        {
                            warn!(%peer_id, ?validator_peer_id, claimed_validator_id = message.validator_id, "Dropping validator message");
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
                    let validator_peer_id = self_.get_validator_cache(message.validator_id).latest().map(|binding| binding.peer_id);
                    // Check that each message actually comes from the peer that it
                    // claims it comes from. Reject it otherwise.
                    if validator_peer_id
                        .as_ref()
                        .map(|pid| *pid != peer_id)
                        .unwrap_or(true)
                    {
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
        let record = self.own_record(validator_address, signing_key_pair);
        self.start_announcing_on_join();
        self.announce_to_validator_peers(&record).await;

        self.network
            .dht_put(validator_address, &record.record, signing_key_pair)
            .await?;

        Ok(())
    }

    async fn disconnect_peer(&self, peer_id: N::PeerId, close_reason: CloseReason) {
        self.network.disconnect_peer(peer_id, close_reason).await
    }

    fn own_record_superseded_at(&self) -> Option<u64> {
        let seen = self.superseded_at.load(Ordering::SeqCst);
        (seen != 0).then_some(seen)
    }

    fn validate_message<TTopic>(&self, id: PubsubId<Self>, acceptance: MsgAcceptance)
    where
        TTopic: Topic + Sync,
    {
        self.network.validate_message::<TTopic>(id, acceptance);
    }

    fn get_peer_id(&self, validator_id: u16) -> Option<<Self::NetworkType as Network>::PeerId> {
        self.get_validator_cache(validator_id)
            .latest()
            .map(|binding| binding.peer_id)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use futures::{stream::BoxStream, FutureExt, StreamExt};
    use nimiq_bls::CompressedPublicKey;
    use nimiq_keys::SecureGenerate;
    use nimiq_network_interface::{
        peer_info::Services,
        request::{MessageMarker, RequestMarker},
    };
    use nimiq_network_mock::{MockHub, MockNetwork, MockPeerId};
    use nimiq_test_log::test;
    use nimiq_utils::tagged_signing::{TaggedKeyPair, TaggedSigned};
    use tokio::sync::Notify;

    use super::*;
    use crate::record::{RecordAnnouncement, RecordsRequest, SignedValidatorRecord};

    type ValNet = ValidatorNetworkImpl<MockNetwork>;
    type Inbox = BoxStream<'static, (RecordAnnouncement<MockPeerId>, MockPeerId)>;

    /// Validator IDs by position in the active set.
    const X: u16 = 0;
    const Y: u16 = 1;
    const Z: u16 = 2;

    fn now_ms() -> u64 {
        (OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000) as u64
    }

    /// A validator of the test's active set.
    struct TestValidator {
        address: Address,
        key: KeyPair,
    }

    impl TestValidator {
        fn new(seed: u8) -> Self {
            Self {
                address: Address::from([seed; 20]),
                key: KeyPair::generate_default_csprng(),
            }
        }

        fn record(&self, peer_id: MockPeerId, timestamp: u64) -> SignedValidatorRecord<MockPeerId> {
            let record = ValidatorRecord::new(peer_id, self.address.clone(), timestamp);
            let signature = self.key.tagged_sign(&record);
            TaggedSigned::new(record, signature)
        }

        fn announcement(
            &self,
            peer_id: MockPeerId,
            timestamp: u64,
        ) -> RecordAnnouncement<MockPeerId> {
            RecordAnnouncement {
                record: self.record(peer_id, timestamp),
            }
        }
    }

    /// The active set in which the `i`-th validator has slot band `i`.
    fn active_set(validators: &[&TestValidator]) -> Validators {
        Validators::new(
            validators
                .iter()
                .map(|validator| {
                    Validator::new(
                        validator.address.clone(),
                        CompressedPublicKey::default(),
                        validator.key.public,
                        0..1,
                    )
                })
                .collect(),
        )
    }

    /// A validator network for `us` with `validators` as the active set.
    fn validator_network(us: Arc<MockNetwork>, validators: &[&TestValidator]) -> ValNet {
        let network = ValNet::new(us);
        network.set_validators(&active_set(validators));
        network
    }

    fn inbox(peer: &MockNetwork) -> Inbox {
        peer.receive_messages::<RecordAnnouncement<MockPeerId>>()
    }

    /// The next announcement delivered to `inbox`, giving the sender a few polls to send it.
    async fn next_announcement(
        inbox: &mut Inbox,
    ) -> Option<(RecordAnnouncement<MockPeerId>, MockPeerId)> {
        for _ in 0..100 {
            if let Some(item) = inbox.next().now_or_never() {
                return item;
            }
            tokio::task::yield_now().await;
        }
        None
    }

    async fn announce(
        from: &MockNetwork,
        to: MockPeerId,
        announcement: RecordAnnouncement<MockPeerId>,
    ) {
        from.message(announcement, to)
            .await
            .expect("the announcement was not accepted");
    }

    /// Waits until `network` resolves `validator_id` to `expected`.
    async fn wait_for_peer_id(network: &ValNet, validator_id: u16, expected: MockPeerId) {
        for _ in 0..200 {
            if network.get_peer_id(validator_id) == Some(expected) {
                return;
            }
            tokio::task::yield_now().await;
        }
        panic!(
            "validator {validator_id} did not resolve to {expected}, got {:?}",
            network.get_peer_id(validator_id)
        );
    }

    /// Waits until `network` reports a copy of our record timestamped `expected`.
    async fn wait_for_superseded_at(network: &ValNet, expected: u64) {
        for _ in 0..200 {
            if network.own_record_superseded_at() == Some(expected) {
                return;
            }
            tokio::task::yield_now().await;
        }
        panic!(
            "the copy of our record was not reported, got {:?}",
            network.own_record_superseded_at()
        );
    }

    /// A mock peer that counts the pulls it receives and, if `answers`, answers them with
    /// `records`.
    struct Responder {
        network: Arc<MockNetwork>,
        pulls: Arc<AtomicUsize>,
    }

    impl Responder {
        fn new(
            hub: &mut MockHub,
            services: Services,
            records: Vec<SignedValidatorRecord<MockPeerId>>,
            answers: bool,
        ) -> Self {
            let address = hub.new_address();
            let network = Arc::new(hub.new_network_with_services(address, services));
            let pulls = Arc::new(AtomicUsize::new(0));
            let mut requests = network.receive_requests::<RecordsRequest<MockPeerId>>();
            let (responder, counter) = (Arc::clone(&network), Arc::clone(&pulls));
            spawn(async move {
                while let Some((_, request_id, _)) = requests.next().await {
                    counter.fetch_add(1, Ordering::Relaxed);
                    if answers {
                        let _ = responder
                            .respond::<RecordsRequest<MockPeerId>>(request_id, records.clone())
                            .await;
                    }
                }
            });
            Self { network, pulls }
        }

        /// A connected validator peer answering with `records`.
        fn validator(
            hub: &mut MockHub,
            us: &MockNetwork,
            records: Vec<SignedValidatorRecord<MockPeerId>>,
        ) -> Self {
            let responder = Self::new(hub, Services::all(), records, true);
            responder.network.dial_mock(us);
            responder
        }

        fn pulls(&self) -> usize {
            self.pulls.load(Ordering::Relaxed)
        }

        /// Waits until the responder received at least one pull.
        async fn pulled(&self) {
            for _ in 0..200 {
                if self.pulls() > 0 {
                    return;
                }
                tokio::task::yield_now().await;
            }
            panic!("no pull arrived");
        }
    }

    /// A fallback that answers `peer_id` and counts how often it is asked.
    fn counting_fallback(peer_id: MockPeerId) -> (Arc<DhtFallback<MockNetwork>>, Arc<AtomicUsize>) {
        let asked = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&asked);
        let fallback: Arc<DhtFallback<MockNetwork>> = Arc::new(move |_| {
            counter.fetch_add(1, Ordering::Relaxed);
            future::ready(Some(peer_id)).boxed()
        });
        (fallback, asked)
    }

    /// Lets spawned tasks run for a while.
    async fn settle() {
        for _ in 0..50 {
            tokio::task::yield_now().await;
        }
    }

    /// A message sent over the validator network in these tests.
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    struct Note(u32);

    impl RequestCommon for Note {
        type Kind = MessageMarker;
        type Response = ();
        const TYPE_ID: u16 = 1;
        const MAX_REQUESTS: u32 = 100;
    }

    /// A request sent over the validator network in these tests, answered with its number.
    #[derive(Clone, Debug, Serialize, Deserialize)]
    struct Ping(u32);

    impl RequestCommon for Ping {
        type Kind = RequestMarker;
        type Response = u32;
        const TYPE_ID: u16 = 2;
        const MAX_REQUESTS: u32 = 100;
    }

    /// Sends `Note(note)` to `to`, claiming to come from `validator_id`.
    async fn send_note(from: &MockNetwork, to: MockPeerId, validator_id: u16, note: u32) {
        let message = ValidatorMessage {
            validator_id,
            inner: Note(note),
        };
        from.message(message, to).await.unwrap();
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

    /// Binds `validator`, at `validator_id`, to `peer_id` through the DHT.
    async fn bind_via_dht(
        network: &ValNet,
        us: &MockNetwork,
        validator: &TestValidator,
        validator_id: u16,
        peer_id: MockPeerId,
        timestamp: u64,
    ) {
        publish_dht(us, validator, peer_id, timestamp).await;
        network.get_peer_id(validator_id);
        wait_for_peer_id(network, validator_id, peer_id).await;
    }

    /// Publishes a DHT record pointing `validator` at `peer_id`. The mock DHT needs a connection.
    async fn publish_dht(
        us: &MockNetwork,
        validator: &TestValidator,
        peer_id: MockPeerId,
        timestamp: u64,
    ) {
        let record = ValidatorRecord::new(peer_id, validator.address.clone(), timestamp);
        us.dht_put(&validator.address, &record, &validator.key)
            .await
            .unwrap();
    }

    #[test(tokio::test)]
    async fn setting_the_public_key_announces_our_record_to_connected_validator_peers() {
        let mut hub = MockHub::default();
        let us = Arc::new(hub.new_network());
        let peer = hub.new_network();
        peer.dial_mock(&us);
        let mut inbox = inbox(&peer);

        let ours = TestValidator::new(0);
        let network = validator_network(Arc::clone(&us), &[]);
        let before = now_ms();
        network
            .set_public_key(&ours.address, &ours.key)
            .await
            .unwrap();

        let (announcement, from) = next_announcement(&mut inbox)
            .await
            .expect("no announcement");
        assert_eq!(from, us.get_local_peer_id());
        let record = &announcement.record.record;
        assert_eq!(record.peer_id, us.get_local_peer_id());
        assert_eq!(record.validator_address, ours.address);
        assert!((before..=now_ms()).contains(&record.timestamp));
        assert!(announcement.record.verify(&ours.key.public));
    }

    #[test(tokio::test)]
    async fn our_record_is_announced_to_a_validator_peer_that_joins_later() {
        let mut hub = MockHub::default();
        let us = Arc::new(hub.new_network());
        let first = hub.new_network();
        first.dial_mock(&us);

        let ours = TestValidator::new(0);
        let network = validator_network(Arc::clone(&us), &[]);
        network
            .set_public_key(&ours.address, &ours.key)
            .await
            .unwrap();

        let later = hub.new_network();
        let mut inbox = inbox(&later);
        later.dial_mock(&us);

        let (announcement, from) = next_announcement(&mut inbox)
            .await
            .expect("no announcement");
        assert_eq!(from, us.get_local_peer_id());
        assert_eq!(announcement.record.record.validator_address, ours.address);
    }

    #[test(tokio::test)]
    async fn our_record_is_not_announced_to_peers_without_the_validator_service() {
        let mut hub = MockHub::default();
        let us = Arc::new(hub.new_network());
        let validator_peer = hub.new_network();
        let address = hub.new_address();
        let full_node = hub.new_network_with_services(address, Services::FULL_BLOCKS);
        validator_peer.dial_mock(&us);
        full_node.dial_mock(&us);
        let mut validator_inbox = inbox(&validator_peer);
        let mut full_node_inbox = inbox(&full_node);

        let ours = TestValidator::new(0);
        let network = validator_network(Arc::clone(&us), &[]);
        network
            .set_public_key(&ours.address, &ours.key)
            .await
            .unwrap();

        assert!(next_announcement(&mut validator_inbox).await.is_some());
        assert!(next_announcement(&mut full_node_inbox).await.is_none());

        // Nor when such a peer joins later.
        let address = hub.new_address();
        let late_full_node = hub.new_network_with_services(address, Services::FULL_BLOCKS);
        let mut late_inbox = inbox(&late_full_node);
        late_full_node.dial_mock(&us);
        assert!(next_announcement(&mut late_inbox).await.is_none());
    }

    #[test(tokio::test)]
    async fn a_new_validator_set_triggers_a_re_announcement() {
        let mut hub = MockHub::default();
        let us = Arc::new(hub.new_network());
        let peer = hub.new_network();
        peer.dial_mock(&us);
        let mut inbox = inbox(&peer);

        let ours = TestValidator::new(0);
        let network = validator_network(Arc::clone(&us), &[]);
        network
            .set_public_key(&ours.address, &ours.key)
            .await
            .unwrap();
        let (first, _) = next_announcement(&mut inbox)
            .await
            .expect("no announcement");

        network.set_validators(&active_set(&[&TestValidator::new(1)]));
        let (again, _) = next_announcement(&mut inbox)
            .await
            .expect("no re-announcement");
        assert_eq!(again.record.record, first.record.record);
    }

    #[test(tokio::test)]
    async fn announcing_does_not_depend_on_the_dht_publish() {
        let mut hub = MockHub::default();
        let us = Arc::new(hub.new_network());
        let ours = TestValidator::new(0);
        let network = validator_network(Arc::clone(&us), &[]);
        // Without a connection, the mock DHT rejects the publish.
        assert!(network
            .set_public_key(&ours.address, &ours.key)
            .await
            .is_err());

        let peer = hub.new_network();
        let mut inbox = inbox(&peer);
        peer.dial_mock(&us);
        assert!(next_announcement(&mut inbox).await.is_some());
    }

    #[test(tokio::test)]
    async fn an_announcement_that_cannot_be_delivered_is_ignored() {
        let mut hub = MockHub::default();
        let us = Arc::new(hub.new_network());
        // An old node: connected, but without a receiver for announcements.
        let old_node = hub.new_network();
        old_node.dial_mock(&us);
        let old_peer_id = old_node.get_local_peer_id();

        let x = TestValidator::new(1);
        publish_dht(&us, &x, old_peer_id, now_ms()).await;
        let network = validator_network(Arc::clone(&us), &[&x]);
        network.get_peer_id(X);
        wait_for_peer_id(&network, X, old_peer_id).await;

        let ours = TestValidator::new(0);
        network
            .set_public_key(&ours.address, &ours.key)
            .await
            .unwrap();
        assert_eq!(network.get_peer_id(X), Some(old_peer_id));
    }

    #[test(tokio::test)]
    async fn a_verified_announcement_binds_the_sender() {
        let mut hub = MockHub::default();
        let us = Arc::new(hub.new_network());
        let peer = hub.new_network();
        peer.dial_mock(&us);

        let x = TestValidator::new(1);
        let network = validator_network(Arc::clone(&us), &[&x]);
        announce(
            &peer,
            us.get_local_peer_id(),
            x.announcement(peer.get_local_peer_id(), now_ms()),
        )
        .await;

        wait_for_peer_id(&network, X, peer.get_local_peer_id()).await;
    }

    #[test(tokio::test)]
    async fn an_unverifiable_announcement_does_not_bind() {
        let mut hub = MockHub::default();
        let us = Arc::new(hub.new_network());
        let peer = hub.new_network();
        let other = hub.new_network();
        peer.dial_mock(&us);
        other.dial_mock(&us);
        let us_peer_id = us.get_local_peer_id();

        let x = TestValidator::new(1);
        let y = TestValidator::new(2);
        let network = validator_network(Arc::clone(&us), &[&x, &y]);

        // Naming another peer, signed with another key, and for an address outside the set.
        let wrong_key = TestValidator {
            address: x.address.clone(),
            key: KeyPair::generate_default_csprng(),
        };
        let stranger = TestValidator::new(3);
        for bad in [
            x.announcement(other.get_local_peer_id(), now_ms()),
            wrong_key.announcement(peer.get_local_peer_id(), now_ms()),
            stranger.announcement(peer.get_local_peer_id(), now_ms()),
        ] {
            announce(&peer, us_peer_id, bad).await;
        }
        // A good one afterwards shows that the bad ones were processed.
        announce(
            &other,
            us_peer_id,
            y.announcement(other.get_local_peer_id(), now_ms()),
        )
        .await;
        wait_for_peer_id(&network, Y, other.get_local_peer_id()).await;

        assert_eq!(network.get_peer_id(X), None);
    }

    #[test(tokio::test)]
    async fn a_newer_announcement_replaces_the_binding_from_the_dht() {
        let mut hub = MockHub::default();
        let us = Arc::new(hub.new_network());
        let old_node = hub.new_network();
        let new_node = hub.new_network();
        old_node.dial_mock(&us);
        new_node.dial_mock(&us);

        let x = TestValidator::new(1);
        let published_at = now_ms();
        publish_dht(&us, &x, old_node.get_local_peer_id(), published_at).await;
        let network = validator_network(Arc::clone(&us), &[&x]);
        network.get_peer_id(X);
        wait_for_peer_id(&network, X, old_node.get_local_peer_id()).await;

        announce(
            &new_node,
            us.get_local_peer_id(),
            x.announcement(new_node.get_local_peer_id(), published_at + 1),
        )
        .await;
        wait_for_peer_id(&network, X, new_node.get_local_peer_id()).await;
    }

    #[test(tokio::test)]
    async fn an_announcement_with_an_older_record_is_ignored() {
        let mut hub = MockHub::default();
        let us = Arc::new(hub.new_network());
        let old_node = hub.new_network();
        let new_node = hub.new_network();
        let other = hub.new_network();
        for peer in [&old_node, &new_node, &other] {
            peer.dial_mock(&us);
        }
        let us_peer_id = us.get_local_peer_id();

        let x = TestValidator::new(1);
        let y = TestValidator::new(2);
        let network = validator_network(Arc::clone(&us), &[&x, &y]);
        let timestamp = now_ms();
        announce(
            &new_node,
            us_peer_id,
            x.announcement(new_node.get_local_peer_id(), timestamp),
        )
        .await;
        wait_for_peer_id(&network, X, new_node.get_local_peer_id()).await;

        announce(
            &old_node,
            us_peer_id,
            x.announcement(old_node.get_local_peer_id(), timestamp - 1),
        )
        .await;
        announce(
            &other,
            us_peer_id,
            y.announcement(other.get_local_peer_id(), timestamp),
        )
        .await;
        wait_for_peer_id(&network, Y, other.get_local_peer_id()).await;

        assert_eq!(network.get_peer_id(X), Some(new_node.get_local_peer_id()));
    }

    #[test(tokio::test)]
    async fn a_repeated_identical_announcement_changes_nothing() {
        let mut hub = MockHub::default();
        let us = Arc::new(hub.new_network());
        let peer = hub.new_network();
        let other = hub.new_network();
        peer.dial_mock(&us);
        other.dial_mock(&us);
        let us_peer_id = us.get_local_peer_id();

        let x = TestValidator::new(1);
        let y = TestValidator::new(2);
        let network = validator_network(Arc::clone(&us), &[&x, &y]);
        let announcement = x.announcement(peer.get_local_peer_id(), now_ms());
        announce(&peer, us_peer_id, announcement.clone()).await;
        wait_for_peer_id(&network, X, peer.get_local_peer_id()).await;

        announce(&peer, us_peer_id, announcement).await;
        announce(
            &other,
            us_peer_id,
            y.announcement(other.get_local_peer_id(), now_ms()),
        )
        .await;
        wait_for_peer_id(&network, Y, other.get_local_peer_id()).await;

        assert_eq!(network.get_peer_id(X), Some(peer.get_local_peer_id()));
    }

    #[test(tokio::test)]
    async fn an_announcement_during_a_resolution_survives_an_older_result() {
        let mut hub = MockHub::default();
        let us = Arc::new(hub.new_network());
        let peer = hub.new_network();
        let listed = hub.new_network();
        peer.dial_mock(&us);

        // The fallback answers only once released, so the resolution stays in flight.
        let release = Arc::new(Notify::new());
        let fallback: Arc<DhtFallback<MockNetwork>> = {
            let release = Arc::clone(&release);
            let listed = listed.get_local_peer_id();
            Arc::new(move |_| {
                let release = Arc::clone(&release);
                async move {
                    release.notified().await;
                    Some(listed)
                }
                .boxed()
            })
        };
        let x = TestValidator::new(1);
        let network = ValNet::new_with_fallback(Arc::clone(&us), fallback);
        network.set_validators(&active_set(&[&x]));
        assert_eq!(network.get_peer_id(X), None);

        announce(
            &peer,
            us.get_local_peer_id(),
            x.announcement(peer.get_local_peer_id(), now_ms()),
        )
        .await;
        wait_for_peer_id(&network, X, peer.get_local_peer_id()).await;

        release.notify_one();
        for _ in 0..100 {
            tokio::task::yield_now().await;
            assert_eq!(network.get_peer_id(X), Some(peer.get_local_peer_id()));
        }
    }

    #[test(tokio::test)]
    async fn a_cache_miss_asks_connected_validator_peers_before_the_dht() {
        let mut hub = MockHub::default();
        let us = Arc::new(hub.new_network());
        let stale = hub.new_network();
        let fresh = hub.new_network();
        let x = TestValidator::new(1);
        let timestamp = now_ms();
        let responder = Responder::validator(
            &mut hub,
            &us,
            vec![x.record(fresh.get_local_peer_id(), timestamp + 1)],
        );
        publish_dht(&us, &x, stale.get_local_peer_id(), timestamp).await;

        let network = validator_network(Arc::clone(&us), &[&x]);
        network.get_peer_id(X);
        wait_for_peer_id(&network, X, fresh.get_local_peer_id()).await;
        assert_eq!(responder.pulls(), 1);
    }

    #[test(tokio::test)]
    async fn the_newest_signed_record_wins_regardless_of_source() {
        let mut hub = MockHub::default();
        let us = Arc::new(hub.new_network());
        let stale = hub.new_network();
        let fresh = hub.new_network();
        let x = TestValidator::new(1);
        let timestamp = now_ms();
        let _responder = Responder::validator(
            &mut hub,
            &us,
            vec![x.record(stale.get_local_peer_id(), timestamp)],
        );
        publish_dht(&us, &x, fresh.get_local_peer_id(), timestamp + 1).await;

        let network = validator_network(Arc::clone(&us), &[&x]);
        network.get_peer_id(X);
        wait_for_peer_id(&network, X, fresh.get_local_peer_id()).await;
    }

    #[test(tokio::test)]
    async fn one_pull_fills_the_bindings_of_every_validator_it_covers() {
        let mut hub = MockHub::default();
        let us = Arc::new(hub.new_network());
        let (x, y, z) = (
            TestValidator::new(1),
            TestValidator::new(2),
            TestValidator::new(3),
        );
        let (px, py, pz) = (hub.new_network(), hub.new_network(), hub.new_network());
        let timestamp = now_ms();
        let responder = Responder::validator(
            &mut hub,
            &us,
            vec![
                x.record(px.get_local_peer_id(), timestamp),
                y.record(py.get_local_peer_id(), timestamp),
                z.record(pz.get_local_peer_id(), timestamp),
            ],
        );

        let network = validator_network(Arc::clone(&us), &[&x, &y, &z]);
        network.get_peer_id(X);
        wait_for_peer_id(&network, X, px.get_local_peer_id()).await;

        // The others are bound without a resolution of their own.
        assert_eq!(network.get_peer_id(Y), Some(py.get_local_peer_id()));
        assert_eq!(network.get_peer_id(Z), Some(pz.get_local_peer_id()));
        settle().await;
        assert_eq!(responder.pulls(), 1);
    }

    #[test(tokio::test)]
    async fn concurrent_misses_share_one_pull() {
        let mut hub = MockHub::default();
        let us = Arc::new(hub.new_network());
        let (x, y) = (TestValidator::new(1), TestValidator::new(2));
        let (px, py) = (hub.new_network(), hub.new_network());
        let timestamp = now_ms();
        let responder = Responder::validator(
            &mut hub,
            &us,
            vec![
                x.record(px.get_local_peer_id(), timestamp),
                y.record(py.get_local_peer_id(), timestamp),
            ],
        );

        let network = validator_network(Arc::clone(&us), &[&x, &y]);
        network.get_peer_id(X);
        network.get_peer_id(Y);
        wait_for_peer_id(&network, X, px.get_local_peer_id()).await;
        wait_for_peer_id(&network, Y, py.get_local_peer_id()).await;
        settle().await;
        assert_eq!(responder.pulls(), 1);
    }

    #[test(tokio::test)]
    async fn pulled_records_are_verified_individually() {
        let mut hub = MockHub::default();
        let us = Arc::new(hub.new_network());
        let (x, y) = (TestValidator::new(1), TestValidator::new(2));
        let (px, py) = (hub.new_network(), hub.new_network());
        let wrong_key = TestValidator {
            address: x.address.clone(),
            key: KeyPair::generate_default_csprng(),
        };
        let timestamp = now_ms();
        let _responder = Responder::validator(
            &mut hub,
            &us,
            vec![
                wrong_key.record(px.get_local_peer_id(), timestamp),
                y.record(py.get_local_peer_id(), timestamp),
            ],
        );

        let network = validator_network(Arc::clone(&us), &[&x, &y]);
        network.get_peer_id(X);
        wait_for_peer_id(&network, Y, py.get_local_peer_id()).await;
        assert_eq!(network.get_peer_id(X), None);
    }

    #[test(tokio::test)]
    async fn a_pull_response_is_bounded_by_the_size_of_the_active_set() {
        let mut hub = MockHub::default();
        let us = Arc::new(hub.new_network());
        let (x, y) = (TestValidator::new(1), TestValidator::new(2));
        let px = hub.new_network();
        let stranger = TestValidator::new(3);
        let timestamp = now_ms();
        // Two validators are active, so only the first two entries count.
        let responder = Responder::validator(
            &mut hub,
            &us,
            vec![
                stranger.record(px.get_local_peer_id(), timestamp),
                stranger.record(px.get_local_peer_id(), timestamp),
                x.record(px.get_local_peer_id(), timestamp),
            ],
        );

        let network = validator_network(Arc::clone(&us), &[&x, &y]);
        network.get_peer_id(X);
        responder.pulled().await;
        settle().await;
        assert_eq!(network.get_peer_id(X), None);
    }

    #[test(tokio::test)]
    async fn a_pull_asks_at_most_a_few_validator_peers_and_no_others() {
        let mut hub = MockHub::default();
        let us = Arc::new(hub.new_network());
        let x = TestValidator::new(1);
        let px = hub.new_network();
        let records = vec![x.record(px.get_local_peer_id(), now_ms())];
        let validators: Vec<Responder> = (0..ValNet::MAX_PULL_PEERS + 1)
            .map(|_| Responder::validator(&mut hub, &us, records.clone()))
            .collect();
        let full_node = Responder::new(&mut hub, Services::FULL_BLOCKS, records.clone(), true);
        full_node.network.dial_mock(&us);

        let network = validator_network(Arc::clone(&us), &[&x]);
        network.get_peer_id(X);
        wait_for_peer_id(&network, X, px.get_local_peer_id()).await;
        settle().await;

        let asked: usize = validators.iter().map(Responder::pulls).sum();
        assert!((1..=ValNet::MAX_PULL_PEERS).contains(&asked), "{asked}");
        assert_eq!(full_node.pulls(), 0);
    }

    #[test(tokio::test)]
    async fn records_from_all_asked_peers_are_merged() {
        let mut hub = MockHub::default();
        let us = Arc::new(hub.new_network());
        let (x, y) = (TestValidator::new(1), TestValidator::new(2));
        let (px, py) = (hub.new_network(), hub.new_network());
        let timestamp = now_ms();
        let knows_x = Responder::validator(
            &mut hub,
            &us,
            vec![x.record(px.get_local_peer_id(), timestamp)],
        );
        let knows_y = Responder::validator(
            &mut hub,
            &us,
            vec![y.record(py.get_local_peer_id(), timestamp)],
        );

        let network = validator_network(Arc::clone(&us), &[&x, &y]);
        network.get_peer_id(X);
        wait_for_peer_id(&network, X, px.get_local_peer_id()).await;
        assert_eq!(network.get_peer_id(Y), Some(py.get_local_peer_id()));
        settle().await;
        assert_eq!((knows_x.pulls(), knows_y.pulls()), (1, 1));
    }

    #[tokio::test(start_paused = true)]
    async fn a_pull_that_is_not_answered_times_out_into_the_dht() {
        let mut hub = MockHub::default();
        let us = Arc::new(hub.new_network());
        let x = TestValidator::new(1);
        let px = hub.new_network();
        let silent = Responder::new(&mut hub, Services::all(), vec![], false);
        silent.network.dial_mock(&us);
        publish_dht(&us, &x, px.get_local_peer_id(), now_ms()).await;

        let network = validator_network(Arc::clone(&us), &[&x]);
        network.get_peer_id(X);
        silent.pulled().await;
        tokio::time::advance(ValNet::PULL_TIMEOUT + Duration::from_millis(1)).await;
        wait_for_peer_id(&network, X, px.get_local_peer_id()).await;
    }

    #[test(tokio::test)]
    async fn the_dht_is_asked_when_no_peer_knows_the_validator() {
        let mut hub = MockHub::default();
        let us = Arc::new(hub.new_network());
        let x = TestValidator::new(1);
        let px = hub.new_network();
        let responder = Responder::validator(&mut hub, &us, vec![]);
        publish_dht(&us, &x, px.get_local_peer_id(), now_ms()).await;

        let network = validator_network(Arc::clone(&us), &[&x]);
        network.get_peer_id(X);
        wait_for_peer_id(&network, X, px.get_local_peer_id()).await;
        assert_eq!(responder.pulls(), 1);
    }

    #[test(tokio::test)]
    async fn the_fallback_is_not_asked_when_a_pull_or_the_dht_answers() {
        let mut hub = MockHub::default();
        let listed = hub.new_network();
        let x = TestValidator::new(1);
        let px = hub.new_network();

        // A pull answers.
        let us = Arc::new(hub.new_network());
        let _responder = Responder::validator(
            &mut hub,
            &us,
            vec![x.record(px.get_local_peer_id(), now_ms())],
        );
        let (fallback, asked) = counting_fallback(listed.get_local_peer_id());
        let network = ValNet::new_with_fallback(Arc::clone(&us), fallback);
        network.set_validators(&active_set(&[&x]));
        network.get_peer_id(X);
        wait_for_peer_id(&network, X, px.get_local_peer_id()).await;
        assert_eq!(asked.load(Ordering::Relaxed), 0);

        // The DHT answers.
        let us = Arc::new(hub.new_network());
        let _responder = Responder::validator(&mut hub, &us, vec![]);
        publish_dht(&us, &x, px.get_local_peer_id(), now_ms()).await;
        let (fallback, asked) = counting_fallback(listed.get_local_peer_id());
        let network = ValNet::new_with_fallback(Arc::clone(&us), fallback);
        network.set_validators(&active_set(&[&x]));
        network.get_peer_id(X);
        wait_for_peer_id(&network, X, px.get_local_peer_id()).await;
        assert_eq!(asked.load(Ordering::Relaxed), 0);
    }

    #[test(tokio::test)]
    async fn the_fallback_is_asked_when_no_signed_source_answers() {
        let mut hub = MockHub::default();
        let us = Arc::new(hub.new_network());
        let listed = hub.new_network();
        let x = TestValidator::new(1);
        let responder = Responder::validator(&mut hub, &us, vec![]);
        let (fallback, asked) = counting_fallback(listed.get_local_peer_id());

        let network = ValNet::new_with_fallback(Arc::clone(&us), fallback);
        network.set_validators(&active_set(&[&x]));
        network.get_peer_id(X);
        wait_for_peer_id(&network, X, listed.get_local_peer_id()).await;
        assert_eq!(asked.load(Ordering::Relaxed), 1);
        assert_eq!(responder.pulls(), 1);
    }

    #[test(tokio::test)]
    async fn a_responder_answers_with_its_newest_signed_record_per_validator_including_its_own() {
        let mut hub = MockHub::default();
        let us = Arc::new(hub.new_network());
        let peer = hub.new_network();
        let (older, newer, py) = (hub.new_network(), hub.new_network(), hub.new_network());
        for node in [&peer, &older, &newer] {
            node.dial_mock(&us);
        }
        let us_peer_id = us.get_local_peer_id();
        let (ours, x, y) = (
            TestValidator::new(0),
            TestValidator::new(1),
            TestValidator::new(2),
        );
        let network = validator_network(Arc::clone(&us), &[&x, &y, &ours]);
        network
            .set_public_key(&ours.address, &ours.key)
            .await
            .unwrap();

        // Two announcements for `x`; `y` is only known through the DHT, which gives no signature.
        let timestamp = now_ms();
        announce(
            &older,
            us_peer_id,
            x.announcement(older.get_local_peer_id(), timestamp),
        )
        .await;
        announce(
            &newer,
            us_peer_id,
            x.announcement(newer.get_local_peer_id(), timestamp + 1),
        )
        .await;
        publish_dht(&us, &y, py.get_local_peer_id(), timestamp).await;
        network.get_peer_id(Y);
        wait_for_peer_id(&network, X, newer.get_local_peer_id()).await;
        wait_for_peer_id(&network, Y, py.get_local_peer_id()).await;

        let mut records = peer
            .request(RecordsRequest::<MockPeerId>::default(), us_peer_id)
            .await
            .unwrap();
        records.sort_by(|a, b| a.record.validator_address.cmp(&b.record.validator_address));
        let summary: Vec<_> = records
            .iter()
            .map(|signed| {
                (
                    signed.record.validator_address.clone(),
                    signed.record.peer_id,
                )
            })
            .collect();
        assert_eq!(
            summary,
            vec![
                (ours.address.clone(), us_peer_id),
                (x.address.clone(), newer.get_local_peer_id()),
            ]
        );
        assert_eq!(records[1].record.timestamp, timestamp + 1);
        assert!(records[0].verify(&ours.key.public));
        assert!(records[1].verify(&x.key.public));
    }

    #[test(tokio::test)]
    async fn a_pulled_record_for_our_own_address_is_never_applied() {
        let mut hub = MockHub::default();
        let us = Arc::new(hub.new_network());
        let peer = hub.new_network();
        let (elsewhere, px) = (hub.new_network(), hub.new_network());
        peer.dial_mock(&us);
        let us_peer_id = us.get_local_peer_id();
        let (ours, x) = (TestValidator::new(0), TestValidator::new(1));
        let timestamp = now_ms();
        let _responder = Responder::validator(
            &mut hub,
            &us,
            vec![
                ours.record(elsewhere.get_local_peer_id(), timestamp + 1000),
                x.record(px.get_local_peer_id(), timestamp),
            ],
        );

        let network = validator_network(Arc::clone(&us), &[&x, &ours]);
        network
            .set_public_key(&ours.address, &ours.key)
            .await
            .unwrap();
        network.get_peer_id(X);
        wait_for_peer_id(&network, X, px.get_local_peer_id()).await;

        // We are not redirected, and we do not relay the copy either.
        assert_ne!(network.get_peer_id(Y), Some(elsewhere.get_local_peer_id()));
        let records = peer
            .request(RecordsRequest::<MockPeerId>::default(), us_peer_id)
            .await
            .unwrap();
        let own: Vec<_> = records
            .iter()
            .filter(|signed| signed.record.validator_address == ours.address)
            .map(|signed| signed.record.peer_id)
            .collect();
        assert_eq!(own, vec![us_peer_id]);
    }

    #[test(tokio::test)]
    async fn a_newer_copy_of_our_own_record_makes_us_re_sign_past_it() {
        let mut hub = MockHub::default();
        let us = Arc::new(hub.new_network());
        let peer = hub.new_network();
        let previous_node = hub.new_network();
        peer.dial_mock(&us);
        let mut inbox = inbox(&peer);
        let ours = TestValidator::new(0);
        // A record of our previous incarnation, whose clock was ahead of ours.
        let ahead = now_ms() + 1000;
        let _responder = Responder::validator(
            &mut hub,
            &us,
            vec![ours.record(previous_node.get_local_peer_id(), ahead)],
        );

        let network = validator_network(Arc::clone(&us), &[&ours]);
        network
            .set_public_key(&ours.address, &ours.key)
            .await
            .unwrap();
        let (first, _) = next_announcement(&mut inbox)
            .await
            .expect("no announcement");
        assert!(first.record.record.timestamp < ahead);
        assert_eq!(network.own_record_superseded_at(), None);

        // Resolving our own address pulls the copy. The network only reports it: the validator
        // holds the key and re-signs by setting the public key again.
        network.get_peer_id(X);
        wait_for_superseded_at(&network, ahead).await;
        assert!(next_announcement(&mut inbox).await.is_none());

        network
            .set_public_key(&ours.address, &ours.key)
            .await
            .unwrap();
        let (again, _) = next_announcement(&mut inbox)
            .await
            .expect("no re-announcement");
        assert_eq!(again.record.record.peer_id, us.get_local_peer_id());
        assert!(again.record.record.timestamp > ahead);
        assert!(again.record.verify(&ours.key.public));
        assert_eq!(network.own_record_superseded_at(), None);
    }

    #[test(tokio::test)]
    async fn messages_are_accepted_only_from_the_bound_peer() {
        let mut hub = MockHub::default();
        let us = Arc::new(hub.new_network());
        let bound = hub.new_network();
        let stranger = hub.new_network();
        bound.dial_mock(&us);
        stranger.dial_mock(&us);
        let us_peer_id = us.get_local_peer_id();

        let x = TestValidator::new(1);
        let network = validator_network(Arc::clone(&us), &[&x]);
        bind_via_dht(&network, &us, &x, X, bound.get_local_peer_id(), now_ms()).await;
        let mut notes = network.receive::<Note>();

        send_note(&stranger, us_peer_id, X, 1).await;
        // A validator ID outside the set.
        send_note(&bound, us_peer_id, Y, 2).await;
        send_note(&bound, us_peer_id, X, 3).await;

        assert_eq!(notes.next().now_or_never(), Some(Some((Note(3), X))));
        assert!(notes.next().now_or_never().is_none());
    }

    #[test(tokio::test)]
    async fn requests_are_accepted_only_from_the_bound_peer() {
        let mut hub = MockHub::default();
        let us = Arc::new(hub.new_network());
        let bound = hub.new_network();
        let stranger = hub.new_network();
        bound.dial_mock(&us);
        stranger.dial_mock(&us);
        let us_peer_id = us.get_local_peer_id();

        let x = TestValidator::new(1);
        let network = validator_network(Arc::clone(&us), &[&x]);
        bind_via_dht(&network, &us, &x, X, bound.get_local_peer_id(), now_ms()).await;
        let mut pings = network.receive_requests::<Ping>();

        // The stranger's request is never answered, so it is only polled once to send it.
        let stranger_request = stranger.request(
            ValidatorMessage {
                validator_id: X,
                inner: Ping(1),
            },
            us_peer_id,
        );
        let mut stranger_request = std::pin::pin!(stranger_request);
        assert!(stranger_request.as_mut().now_or_never().is_none());

        let request = bound.request(
            ValidatorMessage {
                validator_id: X,
                inner: Ping(2),
            },
            us_peer_id,
        );
        let mut request = std::pin::pin!(request);
        assert!(request.as_mut().now_or_never().is_none());
        let (ping, request_id, validator_id) = pings
            .next()
            .now_or_never()
            .flatten()
            .expect("the request was dropped");
        assert_eq!((ping.0, validator_id), (2, X));
        network.respond::<Ping>(request_id, 20).await.unwrap();
        assert_eq!(request.await.unwrap(), 20);
        assert!(pings.next().now_or_never().is_none());
    }

    #[test(tokio::test)]
    async fn a_newer_announcement_moves_acceptance_to_the_new_peer_and_away_from_the_old_one() {
        let mut hub = MockHub::default();
        let us = Arc::new(hub.new_network());
        let old = hub.new_network();
        let new = hub.new_network();
        old.dial_mock(&us);
        new.dial_mock(&us);
        let us_peer_id = us.get_local_peer_id();

        let x = TestValidator::new(1);
        let network = validator_network(Arc::clone(&us), &[&x]);
        let timestamp = now_ms();
        bind_via_dht(&network, &us, &x, X, old.get_local_peer_id(), timestamp).await;
        let mut notes = network.receive::<Note>();
        send_note(&old, us_peer_id, X, 1).await;
        assert_eq!(notes.next().now_or_never(), Some(Some((Note(1), X))));

        announce(
            &new,
            us_peer_id,
            x.announcement(new.get_local_peer_id(), timestamp + 1),
        )
        .await;
        wait_for_peer_id(&network, X, new.get_local_peer_id()).await;

        send_note(&old, us_peer_id, X, 2).await;
        send_note(&new, us_peer_id, X, 3).await;
        assert_eq!(notes.next().now_or_never(), Some(Some((Note(3), X))));
        assert!(notes.next().now_or_never().is_none());
    }

    #[test(tokio::test)]
    async fn an_older_claimant_is_dropped_even_while_connected() {
        let mut hub = MockHub::default();
        let us = Arc::new(hub.new_network());
        let old = hub.new_network();
        let new = hub.new_network();
        old.dial_mock(&us);
        new.dial_mock(&us);
        let us_peer_id = us.get_local_peer_id();

        let x = TestValidator::new(1);
        let network = validator_network(Arc::clone(&us), &[&x]);
        let timestamp = now_ms();
        announce(
            &new,
            us_peer_id,
            x.announcement(new.get_local_peer_id(), timestamp + 1),
        )
        .await;
        wait_for_peer_id(&network, X, new.get_local_peer_id()).await;
        announce(
            &old,
            us_peer_id,
            x.announcement(old.get_local_peer_id(), timestamp),
        )
        .await;
        settle().await;

        let mut notes = network.receive::<Note>();
        send_note(&old, us_peer_id, X, 1).await;
        send_note(&new, us_peer_id, X, 2).await;
        assert_eq!(notes.next().now_or_never(), Some(Some((Note(2), X))));
        assert!(notes.next().now_or_never().is_none());
        assert_eq!(network.get_peer_id(X), Some(new.get_local_peer_id()));
    }

    #[test(tokio::test)]
    async fn a_message_is_sent_to_exactly_one_peer() {
        let mut hub = MockHub::default();
        let us = Arc::new(hub.new_network());
        let old = hub.new_network();
        let new = hub.new_network();
        old.dial_mock(&us);
        new.dial_mock(&us);
        let mut old_inbox = old.receive_messages::<ValidatorMessage<Note>>();
        let mut new_inbox = new.receive_messages::<ValidatorMessage<Note>>();

        let x = TestValidator::new(1);
        let network = validator_network(Arc::clone(&us), &[&x]);
        network.set_validator_id(Some(7));
        let timestamp = now_ms();
        bind_via_dht(&network, &us, &x, X, old.get_local_peer_id(), timestamp).await;
        announce(
            &new,
            us.get_local_peer_id(),
            x.announcement(new.get_local_peer_id(), timestamp + 1),
        )
        .await;
        wait_for_peer_id(&network, X, new.get_local_peer_id()).await;

        network.send_to(X, Note(1)).await.unwrap();

        let (message, from) = new_inbox
            .next()
            .now_or_never()
            .flatten()
            .expect("the new node got nothing");
        assert_eq!(
            (message.validator_id, message.inner, from),
            (7, Note(1), us.get_local_peer_id())
        );
        assert!(old_inbox.next().now_or_never().is_none());
    }

    #[test(tokio::test)]
    async fn get_peer_id_follows_the_binding() {
        let mut hub = MockHub::default();
        let us = Arc::new(hub.new_network());
        let old = hub.new_network();
        let new = hub.new_network();
        old.dial_mock(&us);
        new.dial_mock(&us);

        let x = TestValidator::new(1);
        let network = validator_network(Arc::clone(&us), &[&x]);
        let timestamp = now_ms();
        bind_via_dht(&network, &us, &x, X, old.get_local_peer_id(), timestamp).await;
        assert_eq!(network.get_peer_id(X), Some(old.get_local_peer_id()));

        announce(
            &new,
            us.get_local_peer_id(),
            x.announcement(new.get_local_peer_id(), timestamp + 1),
        )
        .await;
        wait_for_peer_id(&network, X, new.get_local_peer_id()).await;
        assert_eq!(network.get_peer_id(X), Some(new.get_local_peer_id()));
    }

    #[test(tokio::test)]
    async fn a_restarted_validator_is_reached_again_without_the_dht() {
        let mut hub = MockHub::default();
        let us = Arc::new(hub.new_network());
        let old_node = hub.new_network();
        old_node.dial_mock(&us);
        let mut old_inbox = old_node.receive_messages::<ValidatorMessage<Note>>();
        let us_peer_id = us.get_local_peer_id();

        let x = TestValidator::new(1);
        let network = validator_network(Arc::clone(&us), &[&x]);
        network.set_validator_id(Some(7));
        let timestamp = now_ms();
        announce(
            &old_node,
            us_peer_id,
            x.announcement(old_node.get_local_peer_id(), timestamp),
        )
        .await;
        wait_for_peer_id(&network, X, old_node.get_local_peer_id()).await;
        network.send_to(X, Note(1)).await.unwrap();
        assert!(old_inbox.next().now_or_never().flatten().is_some());

        // The validator comes back under a new peer ID. The DHT stays empty throughout.
        old_node.shutdown();
        assert!(network.send_to(X, Note(2)).await.is_err());
        let new_node = hub.new_network();
        let mut new_inbox = new_node.receive_messages::<ValidatorMessage<Note>>();
        new_node.dial_mock(&us);
        announce(
            &new_node,
            us_peer_id,
            x.announcement(new_node.get_local_peer_id(), timestamp + 1),
        )
        .await;
        wait_for_peer_id(&network, X, new_node.get_local_peer_id()).await;

        network.send_to(X, Note(3)).await.unwrap();
        let (message, _) = new_inbox
            .next()
            .now_or_never()
            .flatten()
            .expect("the new node got nothing");
        assert_eq!(message.inner, Note(3));
    }

    #[test(tokio::test)]
    async fn a_transient_failure_binds_the_same_peer_again_when_nothing_newer_exists() {
        let mut hub = MockHub::default();
        let us = Arc::new(hub.new_network());
        let peer = Arc::new(hub.new_network());
        peer.dial_mock(&us);

        let x = TestValidator::new(1);
        let network = validator_network(Arc::clone(&us), &[&x]);
        network.set_validator_id(Some(7));
        announce(
            &peer,
            us.get_local_peer_id(),
            x.announcement(peer.get_local_peer_id(), now_ms()),
        )
        .await;
        wait_for_peer_id(&network, X, peer.get_local_peer_id()).await;

        // The peer does not answer requests yet, so one fails and drops the binding.
        assert!(matches!(
            network.request(Ping(1), X).await,
            Err(NetworkError::Request(_))
        ));

        // Now it does. No source points elsewhere, so the record we hold binds it again.
        answer_pings(Arc::clone(&peer));
        let mut answer = None;
        for _ in 0..200 {
            if let Ok(value) = network.request(Ping(2), X).await {
                answer = Some(value);
                break;
            }
            tokio::task::yield_now().await;
        }
        assert_eq!(answer, Some(2));
    }
}
