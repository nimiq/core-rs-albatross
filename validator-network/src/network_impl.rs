use std::{
    collections::{btree_map::Entry, BTreeMap},
    pin::Pin,
    sync::Arc,
    time::Duration,
};

use async_trait::async_trait;
use beserial::{Deserialize, Serialize};
use futures::{future::join_all, lock::Mutex, Stream, StreamExt};

use nimiq_bls::{CompressedPublicKey, PublicKey, SecretKey, Signature};
use nimiq_network_interface::{message::Message, network::Network, network::Topic, peer::Peer};
use nimiq_utils::tagged_signing::TaggedSignable;

use super::{MessageStream, NetworkError, ValidatorNetwork};

// Helper to get PeerId type from a network
type PeerId<N> = <<N as Network>::PeerType as Peer>::Id;

//struct ValidatorPeerId<TPeerId: Serialize>(TPeerId);

// TODO: Use a tagged signature for validator records
impl<TPeerId> TaggedSignable for ValidatorRecord<TPeerId>
where
    TPeerId: Serialize + Deserialize,
{
    const TAG: u8 = 0x03;
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct ValidatorRecord<TPeerId>
where
    TPeerId: Serialize + Deserialize,
{
    pub peer_id: TPeerId,
    //public_key: PublicKey,
    // TODO: other info?
}

impl<TPeerId> ValidatorRecord<TPeerId>
where
    TPeerId: Serialize + Deserialize,
{
    pub fn new(peer_id: TPeerId) -> Self {
        Self { peer_id }
    }

    pub fn sign(self, secret_key: &SecretKey) -> SignedValidatorRecord<TPeerId> {
        let data = self.serialize_to_vec();
        let signature = secret_key.sign(&data);

        SignedValidatorRecord {
            record: self,
            signature,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct SignedValidatorRecord<TPeerId>
where
    TPeerId: Serialize + Deserialize,
{
    pub record: ValidatorRecord<TPeerId>,
    pub signature: Signature,
}

impl<TPeerId> SignedValidatorRecord<TPeerId>
where
    TPeerId: Serialize + Deserialize,
{
    pub fn verify(&self, public_key: &PublicKey) -> bool {
        public_key.verify(&self.record.serialize_to_vec(), &self.signature)
    }
}

#[derive(Clone, Debug)]
pub struct State<TPeerId> {
    validator_keys: Vec<CompressedPublicKey>,
    validator_peer_id_cache: BTreeMap<CompressedPublicKey, TPeerId>,
}

#[derive(Debug)]
pub struct ValidatorNetworkImpl<N>
where
    N: Network,
    <<N as Network>::PeerType as Peer>::Id: Send + Sync + Serialize + Deserialize,
{
    network: Arc<N>,
    state: Mutex<State<PeerId<N>>>,
}

impl<N> ValidatorNetworkImpl<N>
where
    N: Network,
    <<N as Network>::PeerType as Peer>::Id: Send + Sync + Serialize + Deserialize + Clone,
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

    /// Looks up the peer ID for a validator public key in the DHT.
    async fn resolve_peer_id(
        network: &N,
        public_key: &CompressedPublicKey,
    ) -> Result<Option<PeerId<N>>, NetworkError<N::Error>> {
        if let Some(record) = network
            .dht_get::<_, SignedValidatorRecord<PeerId<N>>>(&public_key)
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
    ) -> Result<PeerId<N>, NetworkError<N::Error>> {
        let mut state = self.state.lock().await;

        let public_key = state
            .validator_keys
            .get(validator_id)
            .ok_or(NetworkError::UnknownValidator(validator_id))?
            .clone();

        let entry = state.validator_peer_id_cache.entry(public_key.clone());

        match entry {
            Entry::Occupied(occupied) => Ok(occupied.get().clone()),
            Entry::Vacant(vacant) => {
                if let Some(peer_id) = Self::resolve_peer_id(&self.network, &public_key).await? {
                    Ok(vacant.insert(peer_id).clone())
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
    <<N as Network>::PeerType as Peer>::Id: Send + Sync + Serialize + Deserialize + Clone,
    <N as Network>::Error: Send,
{
    type Error = NetworkError<<N as Network>::Error>;
    type PeerType = <N as Network>::PeerType;
    type PubsubId = <N as Network>::PubsubId;

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

    async fn get_validator_peer(
        &self,
        validator_id: usize,
    ) -> Result<Option<Arc<<N as Network>::PeerType>>, Self::Error> {
        let peer_id = self.get_validator_peer_id(validator_id).await?;
        Ok(self.network.get_peer(peer_id))
    }

    async fn send_to<M: Message>(
        &self,
        validator_ids: &[usize],
        msg: &M,
    ) -> Vec<Result<(), Self::Error>> {
        let futures = validator_ids
            .iter()
            .copied()
            .map(|validator_id| async move {
                self.get_validator_peer(validator_id)
                    .await?
                    .ok_or(NetworkError::UnknownValidator(validator_id))?
                    .send(msg)
                    .await
                    .map_err(NetworkError::Send)?;
                Ok(())
            });

        join_all(futures)
            .await
            .into_iter()
            .collect::<Vec<Result<(), Self::Error>>>()
    }

    fn receive<M: Message>(&self) -> MessageStream<M, PeerId<N>> {
        Box::pin(
            self.network
                .receive_from_all()
                .map(|(message, peer)| (message, peer.id())),
        )
    }

    async fn publish<TTopic>(&self, topic: &TTopic, item: TTopic::Item) -> Result<(), Self::Error>
    where
        TTopic: Topic + Sync,
    {
        self.network.publish(topic, item).await?;
        Ok(())
    }

    async fn subscribe<TTopic>(
        &self,
        topic: &TTopic,
    ) -> Result<Pin<Box<dyn Stream<Item = (TTopic::Item, Self::PubsubId)> + Send>>, Self::Error>
    where
        TTopic: Topic + Sync,
    {
        Ok(self.network.subscribe(topic).await?)
    }

    fn cache<M: Message>(&self, _buffer_size: usize, _lifetime: Duration) {
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

    async fn validate_message(&self, id: Self::PubsubId) -> Result<bool, Self::Error> {
        self.network.validate_message(id).await.map_err(|err| NetworkError::Network(err))
    }
}
