use std::{collections::BTreeMap, error::Error, sync::Arc};

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

use super::{MessageStream, NetworkError, ValidatorNetwork};
use crate::validator_record::ValidatorRecord;

/// Validator Network state
#[derive(Clone, Debug)]
pub struct State<TPeerId> {
    /// Own validator ID if active, `None` otherwise.
    own_validator_id: Option<u16>,
    /// Set of public keys for each of the validators
    validator_keys: Vec<LazyPublicKey>,
    /// Cache for mapping validator public keys to peer IDs
    validator_peer_id_cache: BTreeMap<CompressedPublicKey, TPeerId>,
}

/// Validator Network implementation
#[derive(Debug)]
pub struct ValidatorNetworkImpl<N>
where
    N: Network,
    N::PeerId: Serialize + Deserialize,
{
    /// A reference to the network containing all peers
    network: Arc<N>,
    /// Internal state
    state: Arc<RwLock<State<N::PeerId>>>,
}

impl<N> ValidatorNetworkImpl<N>
where
    N: Network,
    N::PeerId: Serialize + Deserialize,
{
    pub fn new(network: Arc<N>) -> Self {
        Self {
            network,
            state: Arc::new(RwLock::new(State {
                own_validator_id: None,
                validator_keys: vec![],
                validator_peer_id_cache: BTreeMap::new(),
            })),
        }
    }

    /// For use in closures, so that no reference to `self` needs to be kept around.
    fn arc_clone(&self) -> ValidatorNetworkImpl<N> {
        ValidatorNetworkImpl {
            network: Arc::clone(&self.network),
            state: Arc::clone(&self.state),
        }
    }

    /// Returns the local validator ID, if elected, `Err(NotElected)` otherwise.
    fn local_validator_id<T: Error + 'static>(&self) -> Result<u16, NetworkError<T>> {
        self.state
            .read()
            .own_validator_id
            .ok_or(NetworkError::NotElected)
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

    /// Look up the peer ID for a validator ID.
    async fn get_validator_peer_id(
        &self,
        validator_id: u16,
    ) -> Result<N::PeerId, NetworkError<N::Error>> {
        let (peer_id, public_key) = {
            let state = self.state.read();

            let public_key = state
                .validator_keys
                .get(usize::from(validator_id))
                .ok_or(NetworkError::UnknownValidator(validator_id))?
                .clone();

            let peer_id = state
                .validator_peer_id_cache
                .get(&public_key.compressed().clone())
                .cloned();
            (peer_id, public_key)
        };

        if let Some(peer_id) = peer_id {
            Ok(peer_id)
        } else if let Some(peer_id) = Self::resolve_peer_id(&self.network, &public_key).await? {
            let mut state = self.state.write();
            state
                .validator_peer_id_cache
                .insert(public_key.compressed().clone(), peer_id);
            Ok(peer_id)
        } else {
            log::error!(
                "Could not find peer ID for validator in DHT: public_key = {:?}",
                public_key
            );
            Err(NetworkError::UnknownValidator(validator_id))
        }
    }

    /// Clears the validator->peer_id cache.
    /// The cached entry should be cleared when the peer id might have changed.
    fn clear_validator_peer_id_cache(&self, validator_id: u16) {
        let state = &mut *self.state.write();
        let validator_key = state.validator_keys.get(usize::from(validator_id));
        if let Some(validator_key) = validator_key {
            state
                .validator_peer_id_cache
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
{
    type Error = NetworkError<N::Error>;
    type NetworkType = N;
    type PubsubId = N::PubsubId;

    fn set_validator_id(&self, validator_id: Option<u16>) {
        self.state.write().own_validator_id = validator_id;
    }

    /// Tells the validator network the validator keys for the current set of active validators. The keys must be
    /// ordered, such that the k-th entry is the validator with ID k.
    async fn set_validators(&self, validator_keys: Vec<LazyPublicKey>) {
        log::trace!(
            "setting Validators for ValidatorNetwork: {:?}",
            &validator_keys
        );
        // Create new peer ID cache, but keep validators that are still active.
        let mut state = self.state.write();

        let mut keep_cached = BTreeMap::new();
        for validator_key in &validator_keys {
            if let Some(peer_id) = state
                .validator_peer_id_cache
                .remove(validator_key.compressed())
            {
                keep_cached.insert(validator_key.compressed().clone(), peer_id);
            }
        }

        state.validator_keys = validator_keys;
        state.validator_peer_id_cache = keep_cached;
    }

    async fn send_to<M: Message>(&self, validator_id: u16, msg: M) -> Result<(), Self::Error> {
        let msg = ValidatorMessage {
            validator_id: self.local_validator_id()?,
            inner: msg,
        };
        let peer_id = self
            .get_validator_peer_id(validator_id)
            .await
            .map_err(|error| {
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
        if let Ok(peer_id) = self.get_validator_peer_id(validator_id).await {
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
                        let validator_peer_id = self_.get_validator_peer_id(message.validator_id).await.ok();
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
    ) -> Result<BoxStream<'a, (TTopic::Item, Self::PubsubId)>, Self::Error>
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

    fn validate_message<TTopic>(&self, id: Self::PubsubId, acceptance: MsgAcceptance)
    where
        TTopic: Topic + Sync,
    {
        self.network.validate_message::<TTopic>(id, acceptance);
    }
}
