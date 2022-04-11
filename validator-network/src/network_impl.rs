use std::{
    collections::{btree_map::Entry, BTreeMap},
    sync::Arc,
    time::Duration,
};

use async_trait::async_trait;
use futures::{future::join_all, lock::Mutex, stream::BoxStream, StreamExt};

use beserial::{Deserialize, Serialize};
use nimiq_bls::{CompressedPublicKey, SecretKey};
use nimiq_network_interface::{
    network::{MsgAcceptance, Network, Topic},
    prelude::NetworkEvent,
    request::Request,
};

use super::{MessageStream, NetworkError, ValidatorNetwork};
use crate::validator_record::{SignedValidatorRecord, ValidatorRecord};

#[derive(Clone, Debug)]
pub struct State<TPeerId> {
    validator_keys: Vec<CompressedPublicKey>,
    validator_peer_id_cache: BTreeMap<CompressedPublicKey, TPeerId>,
}

#[derive(Debug)]
pub struct ValidatorNetworkImpl<N>
where
    N: Network,
    N::PeerId: Serialize + Deserialize,
{
    network: Arc<N>,
    state: Mutex<State<N::PeerId>>,
}

impl<N> ValidatorNetworkImpl<N>
where
    N: Network,
    N::PeerId: Serialize + Deserialize,
{
    pub fn new(network: Arc<N>) -> Self {
        Self {
            network,
            state: Mutex::new(State {
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
                    Some(Ok(NetworkEvent::PeerJoined(joined_id))) if joined_id == peer_id => {
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
        public_key: &CompressedPublicKey,
    ) -> Result<Option<N::PeerId>, NetworkError<N::Error>> {
        if let Some(record) = network
            .dht_get::<_, SignedValidatorRecord<N::PeerId>>(&public_key)
            .await?
        {
            if record.verify(&public_key.uncompress().unwrap()) {
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
        let mut state = self.state.lock().await;

        let public_key = state
            .validator_keys
            .get(validator_id)
            .ok_or(NetworkError::UnknownValidator(validator_id))?
            .clone();

        let entry = state.validator_peer_id_cache.entry(public_key.clone());

        match entry {
            Entry::Occupied(occupied) => Ok(*occupied.get()),
            Entry::Vacant(vacant) => {
                if let Some(peer_id) = Self::resolve_peer_id(&self.network, &public_key).await? {
                    Ok(*vacant.insert(peer_id))
                } else {
                    log::error!(
                        "Could not find peer ID for validator in DHT: public_key = {:?}",
                        public_key
                    );
                    Err(NetworkError::UnknownValidator(validator_id))
                }
            }
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
    async fn set_validators(&self, validator_keys: Vec<CompressedPublicKey>) {
        log::trace!(
            "setting Validators for ValidatorNetwork: {:?}",
            &validator_keys
        );
        // Create new peer ID cache, but keep validators that are still active.
        let mut state = self.state.lock().await;

        let mut keep_cached = BTreeMap::new();
        for validator_key in &validator_keys {
            if let Some(peer_id) = state.validator_peer_id_cache.remove(validator_key) {
                keep_cached.insert(validator_key.clone(), peer_id);
            }
        }

        state.validator_keys = validator_keys;
        state.validator_peer_id_cache = keep_cached;
    }

    async fn send_to<Req: Request + Clone>(
        &self,
        validator_ids: &[usize],
        msg: Req,
    ) -> Vec<Result<(), Self::Error>> {
        let futures = validator_ids
            .iter()
            .copied()
            .map(|validator_id| (validator_id, msg.clone()))
            .map(|(validator_id, msg)| async move {
                // previously, there was an `Option<>` here. why?
                let peer_id = if let Ok(peer_id) = self.get_validator_peer_id(validator_id).await {
                    // The peer was cached so the send is fast tracked
                    peer_id
                } else {
                    // The peer could not be retrieved so we update the cache with a fresh lookup
                    let mut state = self.state.lock().await;

                    // get the public key for the validator_id, return NetworkError::UnknownValidator if it does not exist
                    let public_key = state
                        .validator_keys
                        .get(validator_id)
                        .ok_or(NetworkError::UnknownValidator(validator_id))?
                        .clone();

                    // resolve the public key to the peer_id using the DHT record
                    if let Some(peer_id) = Self::resolve_peer_id(&self.network, &public_key).await? {
                        // set the cache with he new peer_id for this public key
                        state
                            .validator_peer_id_cache
                            .insert(public_key.clone(), peer_id);

                        // try to get the peer for the peer_id. If it does not exist it should be dialed
                        if !self.network.has_peer(peer_id) {
                            log::debug!("Not connected to validator {} @ {:?}, dialing...", validator_id, peer_id);
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
                // We don't care about the response. Just do the request to send a message to the peer.
                let _ = self.network.request::<Req, ()>(msg.clone(), peer_id).await.map_err(NetworkError::Request)?;
                Ok(())
            });

        join_all(futures)
            .await
            .into_iter()
            .collect::<Vec<Result<(), Self::Error>>>()
    }

    fn receive<Req: Request + Clone>(&self) -> MessageStream<Req, N::PeerId> {
        let network = Arc::clone(&self.network);
        Box::pin(
            network
                .receive_requests::<Req>()
                .map(move |(message, _request_id, peer_id)| (message, peer_id)),
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

    fn cache<Req: Request>(&self, _buffer_size: usize, _lifetime: Duration) {
        unimplemented!()
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
