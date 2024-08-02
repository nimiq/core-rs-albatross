use std::{
    collections::HashSet,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll, Waker},
};

use futures::{
    future::{BoxFuture, FutureExt},
    stream::{FuturesUnordered, Stream, StreamExt},
};
use linked_hash_map::LinkedHashMap;
use nimiq_block::Block;
use nimiq_blockchain::Blockchain;
use nimiq_blockchain_interface::AbstractBlockchain;
use nimiq_consensus::consensus::{
    consensus_proxy::ConsensusProxy, ResolveBlockError as ConsensusResolveBlockError,
};
use nimiq_keys::Ed25519Signature as SchnorrSignature;
use nimiq_network_interface::network::{CloseReason, MsgAcceptance, Network, PubsubId as _, Topic};
use nimiq_primitives::policy::Policy;
use nimiq_serde::Serialize;
use nimiq_tendermint::SignedProposalMessage;
use nimiq_utils::WakerExt as _;
use nimiq_validator_network::{PubsubId, ValidatorNetwork};
use parking_lot::{Mutex, RwLock};

use super::{
    aggregation::tendermint::proposal::{Header, SignedProposal},
    r#macro::ProposalTopic,
};

type ProposalAndPubsubId<TValidatorNetwork> = (
    <ProposalTopic<TValidatorNetwork> as Topic>::Item,
    PubsubId<TValidatorNetwork>,
);

/// Wrapper for `ResolveBlockErrors` with the addition of `Invalid`, used for proposals whose resolve block did in fact resolve,
/// but the second step of the signature verification failed.
enum ResolveBlockError<TValidatorNetwork: ValidatorNetwork> {
    /// While the proposal predecessor was resolved, the signature was invalid.
    Invalid(PubsubId<TValidatorNetwork>),
    /// The proposal predecessor could not be resolved.
    Unresolved(
        ConsensusResolveBlockError<TValidatorNetwork::NetworkType>,
        PubsubId<TValidatorNetwork>,
    ),
}

/// The buffer holding all proposals, which have been received, but were not yet needed. Any peer can at most have
/// one proposal in this buffer.
///
/// Any proposal in this buffer must have a valid signature, given the signer index within the message.
/// On taking these messages out of the buffer (see [ProposalReceiver]) it must be maintained that
///     1. The predecessor exists
///     2. The proposer of the proposal calculated with the predecessor vrf seed was indeed supposed
///         to be who was stated in the original message.
///
/// Any message that fails any of the above should be dropped, however depending on why it failed, additional
/// action may be advised. If the signature was incorrect for the assumed signer, the peer who relayed the
/// message relayed invalid data and should be banned. If the assumed proposer does not match the actual proposer,
/// the assumed proposer may be punished (as he produced faulty data, proven by the signature and the predecessor vrf).
/// If the predecessor is unavailable (even after requesting it from the peer who originated the proposal or relayed
/// the proposal) they both could be banned as they failed to produce proper data upon being asked to do so.
pub(crate) struct ProposalBuffer<TValidatorNetwork: ValidatorNetwork + 'static>
where
    PubsubId<TValidatorNetwork>: std::fmt::Debug + Unpin,
{
    /// The network used to validate messages and disconnect peers if necessary.
    network: Arc<TValidatorNetwork>,

    /// ConsensusProxy used to resolve proposal predecessors if necessary.
    consensus: ConsensusProxy<TValidatorNetwork::NetworkType>,

    /// A LinkedHashMap containing a proposal per peer_id.
    /// A single proposal could thus be in the buffer multiple times, but a single peer cannot have
    /// more than one proposal waiting.
    buffer: LinkedHashMap<
        <TValidatorNetwork::NetworkType as Network>::PeerId,
        ProposalAndPubsubId<TValidatorNetwork>,
    >,

    /// A FuturesUnordered collection of incomplete tasks to disconnect from a peer.
    unresolved_disconnect_futures: FuturesUnordered<BoxFuture<'static, ()>>,

    /// Unordered Collection of all still pending futures of resolve block requests.
    resolve_block_futures: FuturesUnordered<
        BoxFuture<
            'static,
            Result<
                (SignedProposal, PubsubId<TValidatorNetwork>),
                ResolveBlockError<TValidatorNetwork>,
            >,
        >,
    >,

    /// A collection of peers who currently have a resolve block future pending. Meant to maintain that any
    /// given peer can only have a single such request.
    peers_with_resolving_blocks: HashSet<<TValidatorNetwork::NetworkType as Network>::PeerId>,

    /// A waker in case a proposal gets entered into a buffer while some code is
    /// already waiting to get one out of it.
    waker: Option<Waker>,
}

impl<TValidatorNetwork: ValidatorNetwork + 'static> ProposalBuffer<TValidatorNetwork>
where
    PubsubId<TValidatorNetwork>: std::fmt::Debug + Unpin,
{
    /// Creates a new ProposalBuffer, returning the [ProposalSender] and [ProposalReceiver] that share the buffer.
    /// Blockchain, Consensus and Network are necessary to do basic verification and punishments as well as to resolve blocks.
    // Ignoring clippy warning: this return type is on purpose
    #[allow(clippy::new_ret_no_self)]
    pub fn new(
        blockchain: Arc<RwLock<Blockchain>>,
        network: Arc<TValidatorNetwork>,
        consensus: ConsensusProxy<TValidatorNetwork::NetworkType>,
    ) -> (
        ProposalSender<TValidatorNetwork>,
        ProposalReceiver<TValidatorNetwork>,
    ) {
        // Create the shared buffer
        let buffer = Self {
            network: Arc::clone(&network),
            buffer: LinkedHashMap::new(),
            unresolved_disconnect_futures: FuturesUnordered::new(),
            resolve_block_futures: FuturesUnordered::new(),
            peers_with_resolving_blocks: HashSet::default(),
            waker: None,
            consensus,
        };

        //  Create the shared buffer.
        let shared = Arc::new(Mutex::new(buffer));

        // Create both the sender and the receiver using the same shared buffer.
        let sender = ProposalSender {
            shared: Arc::clone(&shared),
            blockchain: Arc::clone(&blockchain),
            network: Arc::clone(&network),
        };
        let receiver = ProposalReceiver {
            shared,
            blockchain,
            network,
        };

        (sender, receiver)
    }

    /// Rejects the message received as pubsub_id, preventing its propagation
    /// and disconnects from the peer who propagated it.
    fn disconnect_and_reject(&mut self, pubsub_id: PubsubId<TValidatorNetwork>) {
        // Retrieve the relayer of the proposal.
        let source = pubsub_id.propagation_source();

        // Don't propagate the proposal
        self.network
            .validate_message::<ProposalTopic<TValidatorNetwork>>(pubsub_id, MsgAcceptance::Reject);

        let network = Arc::clone(&self.network);

        // disconnect the peer
        let future = async move {
            log::warn!(
                peer_id = %source,
                "Banning peer because an invalid proposal was received"
            );

            network
                .disconnect_peer(source, CloseReason::MaliciousPeer)
                .await;
        }
        .boxed();

        // Push the future to the futures unordered
        self.unresolved_disconnect_futures.push(future);
    }

    /// Polls the resolve block futures. It removes all Futures which resulted in an Err return type until a
    /// properly resolved future is found, or until there are no more futures left.
    pub fn poll_resolve_block_futures(
        &mut self,
        cx: &mut Context,
    ) -> Option<ProposalAndPubsubId<TValidatorNetwork>> {
        while let Poll::Ready(Some(result)) = self.resolve_block_futures.poll_next_unpin(cx) {
            match result {
                Ok(proposal_and_id) => {
                    // Proposal is good to go. Remove peer from the map and return.
                    self.peers_with_resolving_blocks
                        .remove(&proposal_and_id.1.propagation_source());
                    return Some(proposal_and_id);
                }
                Err(ResolveBlockError::Invalid(pubsub_id)) => {
                    // Proposal signature is invalid. Remove peer from the map and disconnect him. Reject the proposal.
                    log::debug!(?pubsub_id, "Proposal invalidly signed. Disconnecting the relaying peer and rejecting the proposal.");
                    self.peers_with_resolving_blocks
                        .remove(&pubsub_id.propagation_source());
                    self.disconnect_and_reject(pubsub_id);
                }
                Err(ResolveBlockError::Unresolved(error, pubsub_id)) => {
                    // The proposals validity cannot be determined as the predecessor cannot be resolved.
                    // Do not propagate the proposal, but also do not punish anyone.
                    log::debug!(
                        ?error,
                        ?pubsub_id,
                        "Proposal predecessor could not be resolved."
                    );

                    self.peers_with_resolving_blocks
                        .remove(&pubsub_id.propagation_source());
                    self.network
                        .validate_message::<ProposalTopic<TValidatorNetwork>>(
                            pubsub_id,
                            MsgAcceptance::Ignore,
                        );
                }
            }
        }
        None
    }

    /// Checks one buffered proposal after another until a proposal which can be returned is found. Proposals which cannot be returned are either
    ///  * invalid and thus rejected/ignored
    ///  * have an unresolved predecessor and can thus not be verified. These proposals get removed from the buffer and a future to
    ///    resolve their predecessor is created.
    pub fn poll_proposal(
        &mut self,
        blockchain_arc: Arc<RwLock<Blockchain>>,
    ) -> Option<(SignedProposal, PubsubId<TValidatorNetwork>)> {
        while let Some((_peer, (signed_proposal, pubsub_id))) = self.buffer.pop_front() {
            // Get a read lock of the blockchain.
            let blockchain = blockchain_arc.read();

            if blockchain.block_number() >= signed_proposal.proposal.block_number {
                // Proposal is outdated, drop it and indicate the network to not propagate it.
                self.network
                    .validate_message::<ProposalTopic<TValidatorNetwork>>(
                        pubsub_id,
                        MsgAcceptance::Ignore,
                    );
                continue;
            }

            // Try and retrieve the predecessor of the proposal.
            match blockchain.get_block(&signed_proposal.proposal.parent_hash, false, None) {
                // Macro block predecessors do not make any sense in the presence of micro blocks.
                Ok(Block::Macro(_block)) => {
                    log::debug!(?pubsub_id, "The predecessor block of a macro block cannot be a macro block. Disconnecting the peer.");
                    self.disconnect_and_reject(pubsub_id)
                }
                // Micro block predecessors can be used to verify the signer. If the block itself is good will be checked later.
                Ok(Block::Micro(block)) => {
                    if !signed_proposal.verify_signer_matches_producer(block, &blockchain) {
                        log::debug!(
                            ?pubsub_id,
                            "Verification of signed proposal failed. Disconnecting the peer."
                        );
                        self.disconnect_and_reject(pubsub_id);
                    } else {
                        // No validate message call here, as later in the process more proposal verification happens.
                        return Some((signed_proposal, pubsub_id));
                    }
                }
                Err(_error) => {
                    // The predecessor is not known.
                    // Remove the proposal from the buffer and create a future resolving the predecessor.
                    log::debug!(
                        ?signed_proposal,
                        "Received Proposal with unknown predecessor. Requesting predecessor."
                    );
                    if self
                        .peers_with_resolving_blocks
                        .insert(pubsub_id.propagation_source())
                    {
                        let consensus = self.consensus.clone();
                        let fut = Self::resolve_block(
                            (signed_proposal, pubsub_id),
                            consensus,
                            Arc::clone(&blockchain_arc),
                        )
                        .boxed();
                        self.resolve_block_futures.push(fut);
                    }
                }
            }
        }
        None
    }

    /// Creates a Future which resolves a block and subsequently does the required steps to make sure the proposals signature is sound.
    async fn resolve_block(
        (signed_proposal, pubsub_id): (SignedProposal, PubsubId<TValidatorNetwork>),
        consensus_proxy: ConsensusProxy<TValidatorNetwork::NetworkType>,
        blockchain: Arc<RwLock<Blockchain>>,
    ) -> Result<(SignedProposal, PubsubId<TValidatorNetwork>), ResolveBlockError<TValidatorNetwork>>
    {
        let hash = signed_proposal.proposal.parent_hash.clone();

        consensus_proxy
            .resolve_block(
                signed_proposal.proposal.block_number - 1,
                hash,
                pubsub_id.clone(),
            )
            .map(move |result| {
                match result {
                    Err(error) => Err(ResolveBlockError::<TValidatorNetwork>::Unresolved(
                        error, pubsub_id,
                    )),
                    Ok(predecessor) => {
                        // Make sure the block is the one that was missing.
                        // Predecessor must be a micro block.
                        let predecessor = if let Block::Micro(block) = predecessor {
                            block
                        } else {
                            return Err(ResolveBlockError::Invalid(pubsub_id));
                        };

                        let blockchain = blockchain.read();

                        // Make sure the signer matches the producer. This also performs some basic predecessor checks.
                        if signed_proposal.verify_signer_matches_producer(predecessor, &blockchain)
                        {
                            Ok((signed_proposal, pubsub_id))
                        } else {
                            Err(ResolveBlockError::Invalid(pubsub_id))
                        }
                    }
                }
            })
            .await
    }
}

/// Structure to send a proposal into the proposal Buffer. Signature verification happens immediately given the signers
/// identity in the message.
/// Checking for a known predecessor and it having been signed by the correct proposer, happens on the receiver
/// side as chances are higher to already have received the blocks predecessor later in the process.
pub(crate) struct ProposalSender<TValidatorNetwork: ValidatorNetwork + 'static>
where
    PubsubId<TValidatorNetwork>: std::fmt::Debug + Unpin,
{
    /// The buffer holding all buffered proposals shared with the [ProposalReceiver]
    shared: Arc<Mutex<ProposalBuffer<TValidatorNetwork>>>,

    /// Reference to the blockchain structure. It is necessary to retrieve the validators.
    /// Just storing the validators themselves here is impractical for they may change with a
    /// skip block.
    blockchain: Arc<RwLock<Blockchain>>,

    /// Reference to the network structure. Necessary to validate message for gossipsup as well as
    /// banning peers if necessary.
    network: Arc<TValidatorNetwork>,
}

impl<TValidatorNetwork: ValidatorNetwork + 'static> ProposalSender<TValidatorNetwork>
where
    PubsubId<TValidatorNetwork>: std::fmt::Debug + Unpin,
{
    /// Sends the proposal and PubsubId into the buffer.
    ///
    /// This function may lead to the proposal not actually being admitted into the buffer as the signature may not verify.
    /// in that case appropriate action is taken within the function as well, as there is no return value.
    pub fn send(&self, proposal: ProposalAndPubsubId<TValidatorNetwork>) {
        // Before progressing any further the signature must be verified against the stated proposer of the message.
        // Whether or not that proposer is the correct proposer will be checked when polling the proposal out of the buffer.

        // Take the lock on blockchain to extract the current validators while making sure the height of the
        // proposal is in the future
        let bc = self.blockchain.read();
        if bc.block_number() >= proposal.0.proposal.block_number {
            log::trace!(proposal = ?proposal.0, "Encountered old proposal");
            // The proposal is older than the current height, meaning the macro block was finalized and cannot be changed.
            // The proposal is dropped.
            self.network
                .validate_message::<ProposalTopic<TValidatorNetwork>>(
                    proposal.1,
                    MsgAcceptance::Ignore,
                );
            return;
        }

        // Make sure the proposal is in the current epoch.
        if Policy::election_block_after(bc.block_number()) < proposal.0.proposal.block_number {
            log::trace!(proposal = ?proposal.0, "Encountered proposal from the next epoch");
            // The proposal is in the next epoch. The validators can not be determined for it and as such it must be ignored.
            // The proposal is dropped.
            self.network
                .validate_message::<ProposalTopic<TValidatorNetwork>>(
                    proposal.1,
                    MsgAcceptance::Ignore,
                );
            return;
        }

        // Fetch current validators.
        let validators = bc.current_validators().unwrap();

        // No need to hold the lock as validators would only change with an election block which makes
        // the validity of proposals moot.
        drop(bc);

        // Make sure the given validator id is not outside of the range of valid validator ids for the current validator set.
        if proposal.0.signer as usize > validators.num_validators() {
            // The proposal indicates a nonsensical proposer as the validator id is out of bounds.
            // The proposal is dropped and propagation is stopped.
            log::debug!(proposal = ?proposal.0, "Proposal signer does not exist, IOOB");
            self.network
                .validate_message::<ProposalTopic<TValidatorNetwork>>(
                    proposal.1,
                    MsgAcceptance::Reject,
                );
            return;
        }

        // Get the validator whose id was presented in the proposal message.
        let stated_proposer = validators.get_validator_by_slot_band(proposal.0.signer);

        // Calculate the hash which had been signed.
        let data = SignedProposal::hash(
            &proposal.0.proposal,
            proposal.0.round,
            proposal.0.valid_round,
        )
        .serialize_to_vec();

        // Get the propagation source, as it is going to be required either way the verification goes,
        // either for storing the proposal in the buffer, or to ban the relaying peer.
        let source = proposal.1.propagation_source();

        // Verify the stated signer did in fact sign this proposal.
        if stated_proposer
            .signing_key
            .verify(&proposal.0.signature, &data)
        {
            // Acquire the lock on the shared buffer.
            let mut shared = self.shared.lock();

            // Put the proposal into the buffer, potentially evicting an already existing proposal.
            if let Some((old_proposal, old_pubsub)) = shared.buffer.insert(source, proposal.clone())
            {
                // If a previous proposal existed, print a debug message to log
                log::debug!(?source, ?old_proposal, ?proposal, "Proposal was replaced",);
                // Indicate to not propagate the proposal.
                // This may not be ideal, but the message got evicted before its validity could be checked fully.
                self.network
                    .validate_message::<ProposalTopic<TValidatorNetwork>>(
                        old_pubsub,
                        MsgAcceptance::Ignore,
                    );
            }

            // A new proposal was admitted into the buffer. Potential waiting tasks can be woken.
            shared.waker.wake();
        } else {
            // The signature verification did not succeed.

            // Signal the network, that this specific message is not to be propagated by this node.
            // The message is also not only not interesting, but it is faulty.
            self.network
                .validate_message::<ProposalTopic<TValidatorNetwork>>(
                    proposal.1,
                    MsgAcceptance::Reject,
                );

            // Punish the propagation source of the proposal for propagating invalid proposal messages, i.e ban the peer.
            let nw = Arc::clone(&self.network);

            log::warn!(
                peer_id = %source,
                "Disconnecting peer because proposal signature verification failed"
            );

            let future = async move {
                nw.disconnect_peer(source, CloseReason::MaliciousPeer).await;
            }
            .boxed();

            // Acquire the lock on the shared buffer.
            let mut shared = self.shared.lock();
            // Add the disconnect future to the FuturesUnordered collection.
            shared.unresolved_disconnect_futures.push(future);

            // Wake, as a new future was added and needs to be polled.
            // A new proposal was admitted into the buffer. Potential waiting tasks can be woken.
            shared.waker.wake();
        }
    }
}

pub(crate) struct ProposalReceiver<TValidatorNetwork: ValidatorNetwork + 'static>
where
    PubsubId<TValidatorNetwork>: std::fmt::Debug + Unpin,
{
    /// The buffer holding all buffered proposals shared with the [ProposalSender]
    shared: Arc<Mutex<ProposalBuffer<TValidatorNetwork>>>,

    /// Reference to the blockchain structure. It is necessary to retrieve the proposals predecessor
    /// as well as the validators.
    blockchain: Arc<RwLock<Blockchain>>,

    /// Reference to the network structure. Necessary to validate message for gossipsup as well as
    /// banning peers if necessary.
    network: Arc<TValidatorNetwork>,
}

impl<TValidatorNetwork: ValidatorNetwork + 'static> Stream for ProposalReceiver<TValidatorNetwork>
where
    PubsubId<TValidatorNetwork>: std::fmt::Debug + Unpin,
{
    type Item = SignedProposalMessage<Header<PubsubId<TValidatorNetwork>>, (SchnorrSignature, u16)>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        // Acquire the shared buffer lock.
        let mut shared = self.shared.lock();

        // Poll proposals from the shared buffer.
        let result = shared.poll_proposal(Arc::clone(&self.blockchain));

        // If a Result was produced, return it as the item on the stream.
        if let Some((proposal, pubsub_id)) = result {
            return Poll::Ready(Some(
                proposal.into_tendermint_signed_message(Some(pubsub_id)),
            ));
        }

        // Poll the resolve block futures to see if a proposals predecessor has resolved.
        let result = shared.poll_resolve_block_futures(cx);

        // If a Result was produced, return it as the item on the stream.
        if let Some((proposal, pubsub_id)) = result {
            return Poll::Ready(Some(
                proposal.into_tendermint_signed_message(Some(pubsub_id)),
            ));
        }

        // Poll the unresolved disconnect futures now that the new futures have been added.
        while let Poll::Ready(Some(())) = shared.unresolved_disconnect_futures.poll_next_unpin(cx) {
            // nothing to do for any of the completed futures
        }

        // Before returning, check if the buffer is empty, if so, store a waker.
        if shared.buffer.is_empty() {
            shared.waker.store_waker(cx);
        }
        // The futures unordered do not need checking as they can only populate via send or the buffer not being empty.

        // There is nothing to return, so return Pending
        Poll::Pending
    }
}

impl<TValidatorNetwork: ValidatorNetwork + 'static> Clone for ProposalReceiver<TValidatorNetwork>
where
    PubsubId<TValidatorNetwork>: std::fmt::Debug + Unpin,
{
    fn clone(&self) -> Self {
        Self {
            shared: Arc::clone(&self.shared),
            blockchain: Arc::clone(&self.blockchain),
            network: Arc::clone(&self.network),
        }
    }
}

#[cfg(test)]
mod test {
    use std::{sync::Arc, time::Duration};

    use futures::{FutureExt, StreamExt};
    use nimiq_block::MacroHeader;
    use nimiq_blockchain::Blockchain;
    use nimiq_blockchain_proxy::BlockchainProxy;
    use nimiq_bls::cache::PublicKeyCache;
    use nimiq_consensus::{
        sync::syncer_proxy::SyncerProxy, Consensus, ConsensusEvent, ConsensusProxy,
    };
    use nimiq_keys::{KeyPair as SchnorrKeyPair, PrivateKey as SchnorrPrivateKey};
    use nimiq_network_interface::network::Network as NetworkInterface;
    use nimiq_network_mock::{MockHub, MockNetwork};
    use nimiq_primitives::policy::Policy;
    use nimiq_serde::{Deserialize, Serialize};
    use nimiq_tendermint::{ProposalMessage, SignedProposalMessage};
    use nimiq_test_log::test;
    use nimiq_test_utils::{
        block_production::{TemporaryBlockProducer, SIGNING_KEY},
        test_network::TestNetwork,
    };
    use nimiq_time::{sleep, timeout};
    use nimiq_utils::spawn::spawn;
    use nimiq_validator_network::network_impl::ValidatorNetworkImpl;
    use nimiq_zkp_component::ZKPComponent;
    use parking_lot::{Mutex, RwLock};
    use tokio::select;

    use super::{ProposalAndPubsubId, ProposalBuffer};
    use crate::{
        aggregation::tendermint::proposal::{Header, SignedProposal},
        r#macro::ProposalTopic,
    };

    /// Given a blockchain and a network creates an instance of Consensus.
    async fn consensus<N: NetworkInterface + TestNetwork>(
        blockchain: Arc<RwLock<Blockchain>>,
        net: Arc<N>,
    ) -> Consensus<N> {
        let blockchain_proxy = BlockchainProxy::from(&blockchain);

        let zkp_proxy = ZKPComponent::new(blockchain_proxy.clone(), Arc::clone(&net), None)
            .await
            .proxy();

        let syncer = SyncerProxy::new_history(
            blockchain_proxy.clone(),
            Arc::clone(&net),
            Arc::new(Mutex::new(PublicKeyCache::new(10))),
            net.subscribe_events(),
        )
        .await;

        Consensus::new(blockchain_proxy, net, syncer, 0, zkp_proxy)
    }

    async fn setup() -> (
        ConsensusProxy<MockNetwork>,
        TemporaryBlockProducer,
        TemporaryBlockProducer,
        Arc<MockNetwork>,
        Arc<MockNetwork>,
        SchnorrKeyPair,
    ) {
        // Single Validator signing key, used to sign the proposals.
        let signing_key = SchnorrKeyPair::from(
            SchnorrPrivateKey::deserialize_from_vec(&hex::decode(SIGNING_KEY).unwrap()).unwrap(),
        );

        // Create a Network hub and 2 networks.
        let mut hub = MockHub::default();
        let nw1 = Arc::new(hub.new_network_with_address(0));
        let nw2 = Arc::new(hub.new_network_with_address(1));

        // Create 2 Temporary block producers.
        let producer1 = TemporaryBlockProducer::new();
        let producer2 = TemporaryBlockProducer::new();

        // For Both producers and networks create one consensus each and force establish the consensus on them.
        let consensus1 = consensus(Arc::clone(&producer1.blockchain), Arc::clone(&nw1)).await;
        let consensus2 = consensus(Arc::clone(&producer2.blockchain), Arc::clone(&nw2)).await;

        // Get a proxy for the second consensus, as it will have to go into the ProposalBuffer.
        // As consensus2 itself will be spawned it must be done before that.
        let consensus_proxy = consensus2.proxy();

        // Get event receivers to make sure both consensus instances are connected and in sync
        let mut consensus_events1 = consensus1.subscribe_events().boxed().fuse();
        let mut consensus_events2 = consensus2.subscribe_events().boxed().fuse();

        // At most wait 200 millis for them to connect and sync as there is nothing to do here.
        let mut deadline = sleep(Duration::from_millis(200)).boxed().fuse();

        // Spawn both consensus before connecting them.
        spawn(consensus1);
        spawn(consensus2);

        // connect the networks, such that the consensus can answer each others requests.
        nw1.dial_mock(&nw2);
        let mut established = (false, false);
        loop {
            select! {
                // Any event which is None or Some(Err(_)) must be considered failures.
                event = consensus_events1.next() => {
                    match event {
                        Some(Ok(ConsensusEvent::Established {..})) => {
                            if established.1 {
                                break
                            }
                            established.0 = true;
                        }
                        _ => established.0 = false,
                    }
                }
                event = consensus_events2.next() => {
                    match event {
                        Some(Ok(ConsensusEvent::Established {..})) => {
                            if established.0 {
                                break;
                            }
                            established.1 = true;
                        }
                        _ => established.1 = false,
                    }
                }
                _elapsed = &mut deadline => panic!("Failed to establish consensus!"),
            }
        }

        (consensus_proxy, producer1, producer2, nw1, nw2, signing_key)
    }

    async fn create_proposal_msg(
        nw1: Arc<MockNetwork>,
        nw2: Arc<MockNetwork>,
        signing_key: SchnorrKeyPair,
        macro_header: MacroHeader,
    ) -> ProposalAndPubsubId<ValidatorNetworkImpl<MockNetwork>> {
        // Create the proposal message which chain 2 will receive.
        let proposal_message: ProposalMessage<Header<MacroHeader>> = ProposalMessage {
            proposal: Header(macro_header, None),
            round: 0,
            valid_round: None,
        };

        let data = SignedProposal::hash(
            &proposal_message.proposal.0,
            proposal_message.round,
            proposal_message.valid_round,
        )
        .serialize_to_vec();

        let signed_message = SignedProposalMessage {
            message: proposal_message,
            signature: (signing_key.sign(&data), 0),
        };

        // Send the proposal over gossipsup to get it correctly filled with a pubsup_id
        // First subscribe to the topic on network2. Note that nothing else subscribes to this.
        // Usually the validator does, but none is present.
        let mut proposals = nw2
            .subscribe::<ProposalTopic<ValidatorNetworkImpl<MockNetwork>>>()
            .await
            .expect("subscribe must succeed");
        // Broadcast on nw1
        nw1.publish::<ProposalTopic<ValidatorNetworkImpl<MockNetwork>>>(signed_message.into())
            .await
            .expect("Publishing on the proposal topic must succeed.");
        // receive the proposal
        proposals.next().await.expect("Proposal")
    }

    /// Has a validator creating a batch worth of blocks and a proposal for the macro block.
    /// Also has a secondary proposal buffer that witness micro blocks. Lets the buffer
    /// see the proposal and produce it as it should be verifiable.
    #[test(tokio::test)]
    async fn it_produces_proposal() {
        let (consensus_proxy, producer1, producer2, nw1, nw2, signing_key) = setup().await;

        // Push blocks until before last micro block for both chains
        for _ in 0..Policy::blocks_per_batch() - 1 {
            let block = producer1.next_block(vec![], false);
            producer2.push(block).expect("Pushing blocks must succeed");
        }

        // Push the next block in chain 1 as well
        let macro_block = producer1.next_block(vec![], false);

        // Create the proposal buffer for node 2
        let (proposal_sender, mut proposal_receiver) = ProposalBuffer::new(
            Arc::clone(&producer2.blockchain),
            Arc::new(ValidatorNetworkImpl::new(Arc::clone(&nw2))),
            consensus_proxy,
        );

        // Create a proposal from node 1 for node 2.
        let proposal = create_proposal_msg(
            nw1,
            nw2,
            signing_key,
            macro_block.unwrap_macro_ref().header.clone(),
        )
        .await;

        // Push the proposal into the ProposalBuffer
        proposal_sender.send(proposal);

        // Poll the ProposalBuffer until it produces a proposal. This should resolve the so far unknown predecessor and resolve.
        // Do this with a timeout as it should really only take a single round trip.
        let _signed_proposal = timeout(Duration::from_millis(400), proposal_receiver.next())
            .await
            .expect("proposal buffer should have produced the proposal before the timeout")
            .expect("proposal buffer should have produced the proposal");
    }

    /// Has a validator creating a batch worth of blocks and a proposal for the macro block.
    /// Also has a secondary proposal buffer that witness all but the last micro block. Lets the buffer
    /// see the proposal and makes it resolve the block.
    #[test(tokio::test)]
    async fn it_resolves_block() {
        let (consensus_proxy, producer1, producer2, nw1, nw2, signing_key) = setup().await;

        // Push blocks until before last micro block for both chains
        for _ in 0..Policy::blocks_per_batch() - 2 {
            let block = producer1.next_block(vec![], false);
            producer2.push(block).expect("Pushing blocks must succeed");
        }

        // Push last micro block only in chain 1
        let _predecessor = producer1.next_block(vec![], false);

        // Push the next block in chain 1 as well
        let macro_block = producer1.next_block(vec![], false);

        // Attempt to push the macro block. It should not work as its parent is unknown
        assert!(producer2.push(macro_block.clone()).is_err());

        // Create the proposal buffer for node 2
        let (proposal_sender, mut proposal_receiver) = ProposalBuffer::new(
            Arc::clone(&producer2.blockchain),
            Arc::new(ValidatorNetworkImpl::new(Arc::clone(&nw2))),
            consensus_proxy,
        );

        // Create a proposal from node 1 for node 2.
        let proposal = create_proposal_msg(
            nw1,
            nw2,
            signing_key,
            macro_block.unwrap_macro_ref().header.clone(),
        )
        .await;

        // Push the proposal into the ProposalBuffer
        proposal_sender.send(proposal);

        // Poll the ProposalBuffer until it produces a proposal. This should resolve the so far unknown predecessor and resolve.
        // Do this with a timeout as it should really only take a single round trip.
        let signed_proposal = timeout(Duration::from_millis(400), proposal_receiver.next())
            .await
            .expect("proposal buffer should have resolved the proposal before the timeout")
            .expect("proposal buffer should have resolved the proposal");

        // Make sure the predecessor of the proposal can be retrieved.
        producer2
            .blockchain
            .read()
            .get_block(&signed_proposal.message.proposal.0.parent_hash, false, None)
            .expect("Must be able to get the predecessor after resolving it.");

        // Finally push the macro block the proposal is for. It should not be an Orphan
        producer2
            .push(macro_block)
            .expect("pushing the macro block must work");
    }

    /// Has a validator creating a batch worth of blocks and a proposal for the macro block.
    /// Also has a secondary proposal buffer that witness all but the last two micro blocks. Lets the buffer
    /// see the proposal and makes it resolve the blocks.
    #[test(tokio::test)]
    async fn it_resolves_blocks() {
        let (consensus_proxy, producer1, producer2, nw1, nw2, signing_key) = setup().await;

        // Push blocks until before last micro block for both chains
        for _ in 0..Policy::blocks_per_batch() - 3 {
            let block = producer1.next_block(vec![], false);
            producer2.push(block).expect("Pushing blocks must succeed");
        }

        // Push last micro block only in chain 1
        let _block = producer1.next_block(vec![], false);

        // Push last micro block only in chain 1
        let _predecessor = producer1.next_block(vec![], false);

        // Push the next block in chain 1 as well
        let macro_block = producer1.next_block(vec![], false);

        // Attempt to push the macro block. It should not work as its parent is unknown
        assert!(producer2.push(macro_block.clone()).is_err());

        // Create the proposal buffer for node 2
        let (proposal_sender, mut proposal_receiver) = ProposalBuffer::new(
            Arc::clone(&producer2.blockchain),
            Arc::new(ValidatorNetworkImpl::new(Arc::clone(&nw2))),
            consensus_proxy,
        );

        // Create a proposal from node 1 for node 2.
        let proposal = create_proposal_msg(
            nw1,
            nw2,
            signing_key,
            macro_block.unwrap_macro_ref().header.clone(),
        )
        .await;

        // Push the proposal into the ProposalBuffer
        proposal_sender.send(proposal);

        // Poll the ProposalBuffer until it produces a proposal. This should resolve the so far unknown predecessor and resolve.
        // Do this with a timeout as it should really only take a single round trip.
        let signed_proposal = timeout(Duration::from_millis(400), proposal_receiver.next())
            .await
            .expect("proposal buffer should have resolved the proposal before the timeout")
            .expect("proposal buffer should have resolved the proposal");

        // Make sure the predecessor of the proposal can be retrieved.
        producer2
            .blockchain
            .read()
            .get_block(&signed_proposal.message.proposal.0.parent_hash, false, None)
            .expect("Must be able to get the predecessor after resolving it.");

        // Finally push the macro block the proposal is for. It should not be an Orphan
        producer2
            .push(macro_block)
            .expect("pushing the macro block must work");
    }

    /// Has a validator creating a batch worth of blocks and a proposal for the macro block.
    /// Also has a secondary proposal buffer witness all but the last micro block but let it also see a skip block instead.
    /// Lets the buffer see the proposal and makes it resolve the block.
    #[test(tokio::test)]
    async fn it_resolves_inferior_chain_block() {
        let (consensus_proxy, producer1, producer2, nw1, nw2, signing_key) = setup().await;

        // Push blocks until before last micro block for both chains
        for _ in 0..Policy::blocks_per_batch() - 2 {
            let block = producer1.next_block(vec![], false);
            producer2.push(block).expect("Pushing blocks must succeed");
        }

        // Produce a skip block and push it on chain 2 making it the superior chain.
        let skip_block = producer1.next_block_no_push(vec![], true);
        producer2
            .push(skip_block)
            .expect("Pushing skip block must succeed");

        // Push last micro block only in chain 1 which is now inferior to the chain of node 2.
        let _predecessor = producer1.next_block(vec![], false);

        // Push the next block in chain 1 as well
        let macro_block = producer1.next_block(vec![], false);

        // Attempt to push the macro block. It should not work as its parent is unknown
        assert!(producer2.push(macro_block.clone()).is_err());

        // Create the proposal buffer for node 2
        let (proposal_sender, mut proposal_receiver) = ProposalBuffer::new(
            Arc::clone(&producer2.blockchain),
            Arc::new(ValidatorNetworkImpl::new(Arc::clone(&nw2))),
            consensus_proxy,
        );

        // Create a proposal from node 1 for node 2.
        let proposal = create_proposal_msg(
            nw1,
            nw2,
            signing_key,
            macro_block.unwrap_macro_ref().header.clone(),
        )
        .await;

        // Push the proposal into the ProposalBuffer
        proposal_sender.send(proposal);

        // Poll the ProposalBuffer until it produces a proposal. This should resolve the so far unknown predecessor and resolve.
        // Do this with a timeout as it should really only take a single round trip.
        let signed_proposal = timeout(Duration::from_millis(400), proposal_receiver.next())
            .await
            .expect("proposal buffer should have resolved the proposal before the timeout")
            .expect("proposal buffer should have resolved the proposal");

        // Make sure the predecessor of the proposal can be retrieved.
        producer2
            .blockchain
            .read()
            .get_block(&signed_proposal.message.proposal.0.parent_hash, false, None)
            .expect("Must be able to get the predecessor after resolving it.");

        // Finally push the macro block the proposal is for. It should not be an Orphan
        producer2
            .push(macro_block)
            .expect("pushing the macro block must work");
    }
}
