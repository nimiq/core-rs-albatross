use std::{collections::BTreeMap, sync::Arc};

use async_trait::async_trait;
use futures::{stream::BoxStream, StreamExt, TryFutureExt};
use nimiq_bls::{lazy::LazyPublicKey, CompressedPublicKey, SecretKey};
use nimiq_network_interface::{
    network::{MsgAcceptance, Network, NetworkEvent, SubscribeEvents, Topic},
    request::{Message, Request, RequestCommon},
};
use nimiq_serde::{Deserialize, Serialize};
use parking_lot::RwLock;

use super::{MessageStream, NetworkError, ValidatorNetwork};
use crate::validator_record::{SignedValidatorRecord, ValidatorRecord};

/// Validator Network state
#[derive(Clone, Debug)]
pub struct State<TPeerId> {
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
    state: RwLock<State<N::PeerId>>,
}

impl<N> ValidatorNetworkImpl<N>
where
    N: Network,
    N::PeerId: Serialize + Deserialize,
{
    pub fn new(network: Arc<N>) -> Self {
        Self {
            network,
            state: RwLock::new(State {
                validator_keys: vec![],
                validator_peer_id_cache: BTreeMap::new(),
            }),
        }
    }

    async fn dial_peer(&self, peer_id: N::PeerId) -> Result<(), NetworkError<N::Error>> {
        let mut event_stream = self.network.subscribe_events();

        if self.network.has_peer(peer_id) {
            return Ok(());
        }

        self.network.dial_peer(peer_id).await?;

        let future = async move {
            loop {
                match event_stream.next().await {
                    Some(Ok(NetworkEvent::PeerJoined(joined_id, _))) if joined_id == peer_id => {
                        break Ok(())
                    }
                    Some(Err(_)) | None => break Err(NetworkError::Offline), // TODO Error type?
                    _ => {}
                }
            }
        };

        tokio::time::timeout(Duration::from_secs(5), future)
            .await
            .map_err(|_| NetworkError::Unreachable)?
    }

    /// Looks up the peer ID for a validator public key in the DHT.
    async fn resolve_peer_id(
        network: &N,
        public_key: &LazyPublicKey,
    ) -> Result<Option<N::PeerId>, NetworkError<N::Error>> {
        if let Some(record) = network
            .dht_get::<_, SignedValidatorRecord<N::PeerId>>(public_key.compressed())
            .await?
        {
            if record.verify(&public_key.uncompress().expect("Invalid public key")) {
                Ok(Some(record.record.peer_id))
            } else {
                Ok(None)
            }
        } else {
            Ok(None)
        }
    }

    /// Look up the peer ID for a validator ID.
    async fn get_validator_peer_id(
        &self,
        validator_id: usize,
    ) -> Result<N::PeerId, NetworkError<N::Error>> {
        let (peer_id, public_key) = {
            let state = self.state.read();

            let public_key = state
                .validator_keys
                .get(validator_id)
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

    async fn send_to<M: Message + Clone>(
        &self,
        validator_id: usize,
        msg: M,
    ) -> Result<(), Self::Error> {
        // previously, there was an `Option<>` here. why?
        let peer_id = if let Ok(peer_id) = self.get_validator_peer_id(validator_id).await {
            // The peer was cached so the send is fast tracked
            peer_id
        } else {
            let public_key = {
                // The peer could not be retrieved so we update the cache with a fresh lookup
                let state = self.state.read();

                // get the public key for the validator_id, return NetworkError::UnknownValidator if it does not exist
                state
                    .validator_keys
                    .get(validator_id)
                    .ok_or(NetworkError::UnknownValidator(validator_id))?
                    .clone()
            };

            // resolve the public key to the peer_id using the DHT record
            if let Some(peer_id) = Self::resolve_peer_id(&self.network, &public_key).await? {
                // set the cache with he new peer_id for this public key
                self.state
                    .write()
                    .validator_peer_id_cache
                    .insert(public_key.compressed().clone(), peer_id);

                // try to get the peer for the peer_id. If it does not exist it should be dialed
                if !self.network.has_peer(peer_id) {
                    log::debug!(
                        "Not connected to validator {} @ {:?}, dialing...",
                        validator_id,
                        peer_id
                    );
                    self.dial_peer(peer_id).await?;
                }
                peer_id
            } else {
                log::error!(
                            "send_to failed; Could not find peer ID for validator in DHT: public_key = {:?}",
                            public_key
                        );
                return Err(NetworkError::UnknownValidator(validator_id));
            }
        };
        // We don't care about the response: spawn the request and intentionally dismiss
        // the response
        tokio::spawn({
            let network = Arc::clone(&self.network);
            async move {
                if let Err(error) = network.message(msg.clone(), peer_id).await {
                    log::error!(%peer_id, %error, "could not send request");
                }
            }
        });
        Ok(())
    }

    async fn request<TRequest: Request>(
        &self,
        request: TRequest,
        validator_id: usize,
    ) -> Result<
        <TRequest as RequestCommon>::Response,
        NetworkError<<Self::NetworkType as Network>::Error>,
    > {
        if let Ok(peer_id) = self.get_validator_peer_id(validator_id).await {
            self.network
                .request(request, peer_id)
                .map_err(NetworkError::Request)
                .await
        } else {
            Err(NetworkError::Unreachable)
        }
    }

    fn receive<M>(&self) -> MessageStream<M, N::PeerId>
    where
        M: Message + Clone,
    {
        self.network.receive_messages()
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
        let record = ValidatorRecord::new(peer_id);
        self.network
            .dht_put(public_key, &record.sign(secret_key))
            .await?;

        Ok(())
    }

    fn validate_message<TTopic>(&self, id: Self::PubsubId, acceptance: MsgAcceptance)
    where
        TTopic: Topic + Sync,
    {
        self.network.validate_message::<TTopic>(id, acceptance);
    }
}
