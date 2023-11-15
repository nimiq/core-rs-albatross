use std::{
    pin::Pin,
    sync::Arc,
    task::{Context, Poll, Waker},
};

use futures::{future::BoxFuture, stream::FuturesUnordered, FutureExt, Stream, StreamExt};
use linked_hash_map::LinkedHashMap;
use nimiq_block::Block;
use nimiq_blockchain::Blockchain;
use nimiq_blockchain_interface::AbstractBlockchain;
use nimiq_keys::Signature as SchnorrSignature;
use nimiq_macros::store_waker;
use nimiq_network_interface::network::{CloseReason, MsgAcceptance, Network, PubsubId, Topic};
use nimiq_serde::Serialize;
use nimiq_tendermint::SignedProposalMessage;
use nimiq_validator_network::ValidatorNetwork;
use parking_lot::{Mutex, RwLock};

use super::{
    aggregation::tendermint::proposal::{Header, SignedProposal},
    r#macro::ProposalTopic,
};

type ProposalAndPubsubId<TValidatorNetwork> = (
    <ProposalTopic<TValidatorNetwork> as Topic>::Item,
    <TValidatorNetwork as ValidatorNetwork>::PubsubId,
);

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
    <TValidatorNetwork as ValidatorNetwork>::PubsubId: std::fmt::Debug + Unpin,
{
    /// A LinkedHashMap containing a proposal per peer_id.
    /// A single proposal could thus be in the buffer multiple times, but a single peer cannot have
    /// more than once proposal waiting.
    buffer: LinkedHashMap<
        <TValidatorNetwork::NetworkType as Network>::PeerId,
        ProposalAndPubsubId<TValidatorNetwork>,
    >,

    /// A FuturesUnordered collection of incomplete tasks to disconnect from a peer.
    unresolved_futures: FuturesUnordered<BoxFuture<'static, ()>>,

    /// A waker in case a proposal gets entered into a buffer while some code is
    /// already waiting to get one out of it.
    proposal_waker: Option<Waker>,

    /// An additional waker in case a disconnect future is added while some code is
    /// waiting to get a proposal.
    ///
    /// The additional waker makes sense, as otherwise the waker would have to be stored for both
    /// the proposal buffer being empty or the unresolved futures being empty. Ideally the
    /// unresolved futures would most of the time be empty as other nodes send validly signed proposals.
    /// With only one waker that would lead to every poll storing the waker, because the futures are empty,
    /// while every addition of a proposal will trigger wake unnecessarily.
    /// The proposals operations in the [ProposalReceiver] are not cheap, as they do read the predecessor.
    disconnect_waker: Option<Waker>,
}

impl<TValidatorNetwork: ValidatorNetwork + 'static> ProposalBuffer<TValidatorNetwork>
where
    <TValidatorNetwork as ValidatorNetwork>::PubsubId: std::fmt::Debug + Unpin,
{
    /// Creates a new ProposalBuffer, returning the [ProposalSender] and [ProposalReceiver] that share the buffer.
    /// Blockchain and Network are necessary to do basic verification and punishments.
    // Ignoring clippy warning: this return type is on purpose
    #[allow(clippy::new_ret_no_self)]
    pub fn new(
        blockchain: Arc<RwLock<Blockchain>>,
        network: Arc<TValidatorNetwork>,
    ) -> (
        ProposalSender<TValidatorNetwork>,
        ProposalReceiver<TValidatorNetwork>,
    ) {
        // Create the shared buffer
        let buffer = Self {
            buffer: LinkedHashMap::new(),
            unresolved_futures: FuturesUnordered::new(),
            proposal_waker: None,
            disconnect_waker: None,
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
}

/// Structure to send a proposal into the proposal Buffer. Signature verification happens immediately given the signers
/// identity in the message.
/// Checking for a known predecessor and it having been signed by the correct proposer, happens on the receiver
/// side as chances are higher to already have received the blocks predecessor later in the process.
pub(crate) struct ProposalSender<TValidatorNetwork: ValidatorNetwork + 'static>
where
    <TValidatorNetwork as ValidatorNetwork>::PubsubId: std::fmt::Debug + Unpin,
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
    <TValidatorNetwork as ValidatorNetwork>::PubsubId: std::fmt::Debug + Unpin,
{
    /// Sends the proposal and PubsubId into the buffer.
    ///
    /// This function may lead to the proposal not actually being admitted into the buffer as the signature may not verify.
    /// in that case appropriate action is taken within the function as well, as there is no return value.
    pub fn send(&self, proposal: ProposalAndPubsubId<TValidatorNetwork>) {
        // Before progressing any further the signature must be verified against the stated proposer of the message.
        // Whether or not that proposer is the correct proposer will be checked when polling the proposal out of the buffer.

        // Fetch current validators and get the validator whose id was presented in the proposal message.
        let validators = self.blockchain.read().current_validators().unwrap();
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
            if let Some(waker) = shared.proposal_waker.take() {
                waker.wake()
            }
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
            let future = async move {
                nw.disconnect_peer(source, CloseReason::MaliciousPeer).await;
            }
            .boxed();

            // Acquire the lock on the shared buffer.
            let mut shared = self.shared.lock();
            // Add the disconnect future to the FuturesUnordered collection.
            shared.unresolved_futures.push(future);

            // Wake, as a new future was added and needs to be polled.
            // A new proposal was admitted into the buffer. Potential waiting tasks can be woken.
            if let Some(waker) = shared.disconnect_waker.take() {
                waker.wake()
            }
        }
    }
}

pub(crate) struct ProposalReceiver<TValidatorNetwork: ValidatorNetwork + 'static>
where
    <TValidatorNetwork as ValidatorNetwork>::PubsubId: std::fmt::Debug + Unpin,
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
    <TValidatorNetwork as ValidatorNetwork>::PubsubId: std::fmt::Debug + Unpin,
{
    type Item = SignedProposalMessage<
        Header<<TValidatorNetwork as ValidatorNetwork>::PubsubId>,
        (SchnorrSignature, u16),
    >;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        // Acquire the shared buffer lock.
        let mut shared = self.shared.lock();

        // Collect futures which need pushing into the futures unordered of the shared buffer.
        // Since the `entries` call down a couple of lines will take a mutable borrow of shared they
        // must be pushed after that loop returned.
        let mut futures = vec![];

        let proposal_and_pubsubid = shared.buffer.entries().find_map(|entry| {
            let (signed_proposal, _pubsub_id) = entry.get();

            // Get a read lock of the blockchain.
            let blockchain = self.blockchain.read();

            // Try and retrieve the predecessor of the proposal.
            match blockchain.get_block(&signed_proposal.proposal.parent_hash, false, None) {
                Ok(Block::Micro(block)) => {
                    // Proposal predecessor is known.
                    // Proceed to make sure the signer, whose signature was already verified,
                    // is in fact also the one who was supposed to produce this proposal.

                    // Get the active validators.
                    let validators = blockchain.current_validators().unwrap();

                    // Get the validator who signed the proposal and whose signature was already verified.
                    let assumed_validator =
                        validators.get_validator_by_slot_band(signed_proposal.signer);

                    // Calculate the validator who was supposed to produce the block.
                    let actual_validator = blockchain
                        .get_proposer_at(
                            signed_proposal.proposal.block_number,
                            signed_proposal.round,
                            block.header.seed.entropy(),
                            None,
                        )
                        .expect("Must be able to calculate block producer.")
                        .validator;

                    // Compare the expected and the actual validator
                    if *assumed_validator == actual_validator {
                        // Validators match. The signature was already verified. Proposal is good to go.
                        // DO NOT validate message the proposal, as there is much more to the validity than the signature.
                        Some(entry.remove())
                    } else {
                        // Assumed validator is not the proposer. Punish the validator (TODO).
                        // Proposal is removed as it is not a valid proposal.
                        log::debug!("Found MacroHeader Proposal where validators do not match.");
                        let (_proposal, id) = entry.remove();

                        // The peer relaying this proposal relayed a faulty proposal, disconnect him.
                        let source = id.propagation_source();
                        let nw = Arc::clone(&self.network);
                        let future = async move {
                            nw.disconnect_peer(source, CloseReason::MaliciousPeer).await;
                        }
                        .boxed();

                        // Push the future to the futures unordered
                        futures.push(future);

                        // Don't propagate the proposal
                        self.network
                            .validate_message::<ProposalTopic<TValidatorNetwork>>(
                                id,
                                MsgAcceptance::Reject,
                            );

                        None
                    }
                }
                Ok(Block::Macro(_)) => {
                    // the proposal points to a macro block which in the presence of micro blocks is impossible.
                    // The originator created a bogus block. Punish the validator(TODO)
                    log::debug!("Found MacroHeader Proposal with a non MicroBlock predecessor.");
                    let (_proposal, id) = entry.remove();

                    // The peer relaying this proposal relayed a faulty proposal, disconnect him.
                    let source = id.propagation_source();
                    let nw = Arc::clone(&self.network);
                    let future = async move {
                        nw.disconnect_peer(source, CloseReason::MaliciousPeer).await;
                    }
                    .boxed();

                    // Push the future to the futures unordered
                    futures.push(future);

                    // Don't propagate the proposal
                    self.network
                        .validate_message::<ProposalTopic<TValidatorNetwork>>(
                            id,
                            MsgAcceptance::Reject,
                        );
                    None
                }
                Err(_) => {
                    // Block is not known. Request it. (TODO)
                    // Do not remove the block, maybe next time the predecessor is there.
                    None
                }
            }
        });

        // Push newly accumulated futures.
        for future in futures {
            shared.unresolved_futures.push(future);
        }

        // Poll the unresolved disconnect futures now that the new futures have been added.
        while let Poll::Ready(Some(())) = shared.unresolved_futures.poll_next_unpin(cx) {
            // nothing to for any of the completed futures
        }

        if let Some((proposal, pubsubid)) = proposal_and_pubsubid {
            Poll::Ready(Some(
                proposal.into_tendermint_signed_message(Some(pubsubid)),
            ))
        } else {
            // Before returning, check if any of the collections are empty, if so, store a waker.
            if shared.buffer.is_empty() {
                store_waker!(shared, proposal_waker, cx);
            }
            if shared.unresolved_futures.is_empty() {
                store_waker!(shared, disconnect_waker, cx);
            }

            // There is nothing to return, so return Pending
            Poll::Pending
        }
    }
}

impl<TValidatorNetwork: ValidatorNetwork + 'static> Clone for ProposalReceiver<TValidatorNetwork>
where
    <TValidatorNetwork as ValidatorNetwork>::PubsubId: std::fmt::Debug + Unpin,
{
    fn clone(&self) -> Self {
        Self {
            shared: Arc::clone(&self.shared),
            blockchain: Arc::clone(&self.blockchain),
            network: Arc::clone(&self.network),
        }
    }
}
