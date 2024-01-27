use std::sync::Arc;

use futures::{
    future::{self, BoxFuture, FutureExt},
    stream::{BoxStream, StreamExt},
};
use nimiq_block::{Block, MacroBlock, TendermintProof};
use nimiq_blockchain::{BlockProducer, Blockchain};
use nimiq_blockchain_interface::AbstractBlockchain;
use nimiq_collections::BitSet;
use nimiq_handel::{
    aggregation::Aggregation, identity::IdentityRegistry, protocol::Protocol as _,
    verifier::VerificationResult,
};
use nimiq_hash::{Blake2sHash, Hash};
use nimiq_keys::Signature as SchnorrSignature;
use nimiq_primitives::{
    policy::Policy, slots_allocation::Validators, TendermintIdentifier, TendermintStep,
    TendermintVote,
};
use nimiq_serde::Serialize;
use nimiq_tendermint::{
    Proposal, ProposalError, ProposalMessage, Protocol, ProtocolError, SignedProposalMessage, Step,
    TaggedAggregationMessage,
};
use nimiq_validator_network::{
    single_response_requester::SingleResponseRequester, PubsubId, ValidatorNetwork,
};
use parking_lot::RwLock;

use crate::{
    aggregation::{
        registry::ValidatorRegistry,
        tendermint::{
            contribution::{AggregateMessage, TendermintContribution},
            proposal::{Body, Header, RequestProposal, SignedProposal},
            protocol::TendermintAggregationProtocol,
            update_message::TendermintUpdate,
        },
    },
    r#macro::ProposalTopic,
};

// A note for the signing of the proposal:
// There are two distinct signatures for any given proposal.
// The first one is a Schnorr signature from the proposer of any given round over the header hash.
//     This hash does NOT include the pk_tree_root and this signature is not the one being aggregated.
// The other one is a BLS signature over the zkp_hash, which is defined as Blake2S(Blake2b(header_hash).append(pk_tree_root))
//     This one is being aggregated, and contains the pk_tree_root, so the body is necessary for the verification of the signature.

struct NetworkWrapper<TValidatorNetwork: ValidatorNetwork> {
    network: Arc<TValidatorNetwork>,
    tag: (u32, Step),
    height: u32,
}

impl<TValidatorNetwork: ValidatorNetwork> NetworkWrapper<TValidatorNetwork> {
    fn new(height: u32, tag: (u32, Step), network: Arc<TValidatorNetwork>) -> Self {
        Self {
            height,
            network,
            tag,
        }
    }
}
impl<TValidatorNetwork: ValidatorNetwork + 'static> nimiq_handel::network::Network
    for NetworkWrapper<TValidatorNetwork>
{
    type Contribution = TendermintContribution;

    fn send_to(
        &self,
        (msg, recipient): (nimiq_handel::update::LevelUpdate<Self::Contribution>, u16),
    ) -> futures::future::BoxFuture<'static, ()> {
        // wrap the level update in the AggregateMessage
        let aggregation = AggregateMessage(msg);
        // tag it
        let tagged_aggregation_message = TaggedAggregationMessage {
            tag: self.tag,
            aggregation,
        };
        // and create the update.
        let update_message = TendermintUpdate {
            message: tagged_aggregation_message,
            height: self.height,
        };

        // clone network so it can be moved into the future
        let nw = Arc::clone(&self.network);

        // create the send future and return it.
        async move {
            if let Err(error) = nw.send_to(recipient, update_message).await {
                log::error!(?error, recipient, "Failed to send message");
            }
        }
        .boxed()
    }
}

pub struct TendermintProtocol<TValidatorNetwork: ValidatorNetwork> {
    // The network that is going to be used to communicate with the other validators.
    pub network: Arc<TValidatorNetwork>,
    // The slot band for our validator.
    pub validator_slot_band: u16,
    // The block number of the macro block to produce.
    pub block_height: u32,
    // Information relative to our validator that is necessary to produce blocks.
    pub block_producer: BlockProducer,
    // The validators for the current epoch.
    pub current_validators: Validators,
    // The main blockchain struct. Contains all of this validator information about the current chain.
    pub blockchain: Arc<RwLock<Blockchain>>,
    // Validator registry on the heap for easy cloning into handel protocol.
    validator_registry: Arc<ValidatorRegistry>,
}

impl<TValidatorNetwork: ValidatorNetwork> Clone for TendermintProtocol<TValidatorNetwork> {
    fn clone(&self) -> Self {
        Self {
            network: Arc::clone(&self.network),
            validator_slot_band: self.validator_slot_band,
            block_height: self.block_height,
            block_producer: self.block_producer.clone(),
            current_validators: self.current_validators.clone(),
            blockchain: Arc::clone(&self.blockchain),
            validator_registry: Arc::clone(&self.validator_registry),
        }
    }
}

impl<TValidatorNetwork: ValidatorNetwork + 'static> TendermintProtocol<TValidatorNetwork>
where
    PubsubId<TValidatorNetwork>: std::fmt::Debug + Unpin,
{
    pub fn new(
        blockchain: Arc<RwLock<Blockchain>>,
        network: Arc<TValidatorNetwork>,
        block_producer: BlockProducer,
        current_validators: Validators,
        validator_slot_band: u16,
        block_height: u32,
    ) -> Self {
        Self {
            block_producer,
            blockchain,
            block_height,
            validator_slot_band,
            validator_registry: Arc::new(ValidatorRegistry::new(current_validators.clone())),
            current_validators,
            network,
        }
    }
}

impl<TValidatorNetwork: ValidatorNetwork + 'static> Protocol
    for TendermintProtocol<TValidatorNetwork>
where
    PubsubId<TValidatorNetwork>: std::fmt::Debug + Unpin,
{
    type Decision = MacroBlock;
    type Proposal = Header<PubsubId<TValidatorNetwork>>;
    type ProposalHash = Blake2sHash;
    type Inherent = Body;
    type InherentHash = Blake2sHash;
    type Aggregation = TendermintContribution;
    type AggregationMessage = AggregateMessage;
    type ProposalSignature = (SchnorrSignature, u16);

    const F_PLUS_ONE: usize = Policy::F_PLUS_ONE as usize;
    const TWO_F_PLUS_ONE: usize = Policy::TWO_F_PLUS_ONE as usize;
    const TIMEOUT_DELTA: u64 = 1000;
    const TIMEOUT_INIT: u64 = 1000;

    fn is_proposer(&self, round: u32) -> Result<bool, ProtocolError> {
        let blockchain = self.blockchain.read();

        // Abort if the blockchain has changed.
        if blockchain.block_number() != self.block_height - 1 {
            return Err(ProtocolError::Abort);
        }

        // Get best block for preceding micro block.
        // The best block might change, thus the vrf is not stored in separation
        let vrf_seed = match blockchain.get_block_at(self.block_height - 1, false, None) {
            Ok(Block::Micro(block)) => block.header.seed,
            _ => panic!("Preceding block must be a micro block and it must be known."),
        };

        // Get the validator for this round.
        let proposer_slot = blockchain
            .get_proposer(self.block_height, round, vrf_seed.entropy(), None)
            .expect("Couldn't find slot owner!");

        // Check if the slot bands match.
        // TODO Instead of identifying the validator by its slot_band, we should identify it by its
        // address instead.
        Ok(proposer_slot.band == self.validator_slot_band)
    }

    fn create_proposal(
        &self,
        round: u32,
    ) -> Result<(ProposalMessage<Self::Proposal>, Self::Inherent), ProtocolError> {
        let blockchain = self.blockchain.read();

        // Abort if the blockchain state has changed.
        if blockchain.block_number() != self.block_height - 1 {
            return Err(ProtocolError::Abort);
        }

        // Create the proposal.
        let time = blockchain.time.now();
        let block = self
            .block_producer
            .next_macro_block_proposal(&blockchain, time, round, vec![]);

        // Always `Some(…)` because the above function always sets it to `Some(…)`.
        let body = block.body.expect("produced blocks always have a body");

        // Return the block header and body as the proposal.
        Ok((
            ProposalMessage {
                proposal: Header(block.header, None), // Created proposals do not have a PubSubId
                round,
                valid_round: None,
            },
            Body(body),
        ))
    }

    fn broadcast_proposal(
        &self,
        proposal: SignedProposalMessage<Self::Proposal, Self::ProposalSignature>,
    ) {
        let nw = Arc::clone(&self.network);
        tokio::spawn(async move {
            nw.publish::<ProposalTopic<TValidatorNetwork>>(proposal.into())
                .await
        });
    }

    fn request_proposal(
        &self,
        proposal_hash: Self::ProposalHash,
        round_number: u32,
        candidates: BitSet,
    ) -> futures::future::BoxFuture<
        'static,
        Option<SignedProposalMessage<Self::Proposal, Self::ProposalSignature>>,
    > {
        let identity = self.validator_registry.signers_identity(&candidates);
        if identity.is_empty() {
            return future::ready(None).boxed();
        }

        // First out of the signatory slots calculate the signatory validators
        let candidate_peers = identity.as_vec();

        let request = RequestProposal {
            block_number: self.block_height,
            round_number,
            proposal_hash,
        };

        SingleResponseRequester::new(
            Arc::clone(&self.network),
            candidate_peers,
            request,
            (Arc::clone(&self.blockchain), self.block_height),
            3,
            |response, (blockchain, block_height)| {
                if let Some(signed_proposal) = response {
                    let blockchain = blockchain.read();

                    let vrf_seed = match blockchain.get_block(
                        &signed_proposal.proposal.parent_hash,
                        false,
                        None,
                    ) {
                        // Block is known, proceed to verify the producer.
                        Ok(Block::Micro(block)) => block.header.seed,
                        // Block is not known, Cannot verify the proposal
                        _ => return None,
                    };

                    let proposer = blockchain
                        .get_proposer(
                            block_height,
                            signed_proposal.round,
                            vrf_seed.entropy(),
                            None,
                        )
                        .expect("Couldn't find slot owner!")
                        .validator
                        .signing_key;

                    let data = SignedProposal::hash(
                        &signed_proposal.proposal,
                        signed_proposal.round,
                        signed_proposal.valid_round,
                    )
                    .serialize_to_vec();

                    if proposer.verify(&signed_proposal.signature, &data) {
                        return Some(signed_proposal.into_tendermint_signed_message(None));
                    }
                }
                None
            },
        )
        .boxed()
    }

    fn verify_proposal(
        &self,
        proposal: &SignedProposalMessage<Self::Proposal, Self::ProposalSignature>,
        precalculated_inherent: Option<Self::Inherent>,
    ) -> Result<Self::Inherent, ProposalError> {
        // Assemble the proposed header with the body into a MacroBlock.
        let proposed_block = MacroBlock {
            header: proposal.message.proposal.0.clone(),
            body: precalculated_inherent.map(|body| body.0),
            justification: None,
        };

        // verify_macro_block_proposal could create and commit a write transaction, thus take an
        // upgradable read lock on blockchain.
        let blockchain = self.blockchain.upgradable_read();

        // Do the proposal verification.
        blockchain
            .verify_macro_block_proposal(
                proposed_block,
                proposal.message.round,
                proposal.message.valid_round,
            )
            .map(Body)
            .map_err(|error| {
                log::debug!(?error, ?proposal, "Proposal verification failed",);
                ProposalError::InvalidProposal
            })
    }

    fn sign_proposal(
        &self,
        proposal_message: &ProposalMessage<Self::Proposal>,
    ) -> Self::ProposalSignature {
        let data = SignedProposal::hash(
            &proposal_message.proposal.0,
            proposal_message.round,
            proposal_message.valid_round,
        )
        .serialize_to_vec();
        (
            self.block_producer.signing_key.sign(&data),
            self.validator_slot_band,
        )
    }

    fn create_aggregation(
        &self,
        round: u32,
        step: nimiq_tendermint::Step,
        proposal_hash: Option<Self::ProposalHash>,
        update_stream: BoxStream<'static, Self::AggregationMessage>,
    ) -> BoxStream<'static, Self::Aggregation> {
        // Wrap the network
        let network =
            NetworkWrapper::new(self.block_height, (round, step), Arc::clone(&self.network));

        let step = match step {
            Step::Precommit => TendermintStep::PreCommit,
            Step::Prevote => TendermintStep::PreVote,
            _ => panic!("Step must be either prevote or precommit."),
        };

        let id = TendermintIdentifier {
            block_number: self.block_height,
            round_number: round,
            step,
        };

        let tendermint_vote = TendermintVote {
            proposal_hash,
            id: id.clone(),
        };

        let own_contribution = TendermintContribution::from_vote(
            tendermint_vote,
            &self.block_producer.voting_key.secret_key,
            self.validator_registry.get_slots(self.validator_slot_band),
        );

        let protocol = TendermintAggregationProtocol::new(
            Arc::clone(&self.validator_registry),
            self.validator_slot_band as usize,
            1, // to be removed
            id,
        );

        Aggregation::new(
            protocol,
            nimiq_handel::config::Config::default(),
            own_contribution,
            update_stream.map(|item| item.0).boxed(),
            network,
        )
        .boxed()
    }

    fn verify_aggregation_message(
        &self,
        round: u32,
        step: Step,
        message: Self::AggregationMessage,
    ) -> BoxFuture<'static, Result<(), ()>> {
        let step = match step {
            Step::Precommit => TendermintStep::PreCommit,
            Step::Prevote => TendermintStep::PreVote,
            Step::Propose => TendermintStep::Propose,
        };

        let id = TendermintIdentifier {
            block_number: self.block_height,
            round_number: round,
            step,
        };

        let protocol = TendermintAggregationProtocol::new(
            Arc::clone(&self.validator_registry),
            self.validator_slot_band as usize,
            1,
            id,
        );

        async move {
            if matches!(
                protocol.verify(&message.0.aggregate).await,
                VerificationResult::Ok
            ) {
                Ok(())
            } else {
                Err(())
            }
        }
        .boxed()
    }

    fn create_decision(
        &self,
        proposal: Self::Proposal,
        inherent: Self::Inherent,
        aggregation: Self::Aggregation,
        round: u32,
    ) -> Self::Decision {
        // make sure the proof is sufficient for the proposal
        let proof = aggregation
            .contributions
            .get(&Some(proposal.hash()))
            .expect("must have header hash present in aggregate");

        if proof.signers.len() < Policy::TWO_F_PLUS_ONE as usize {
            panic!("Not enough votes to produce a proof")
        } else {
            // make sure the body fits the proposal
            if inherent.0.hash::<Blake2sHash>() == proposal.0.body_root {
                // Assemble the block and return it.
                MacroBlock {
                    header: proposal.0,
                    body: Some(inherent.0),
                    justification: Some(TendermintProof {
                        round,
                        sig: proof.clone(),
                    }),
                }
            } else {
                panic!("Body hash mismatch!");
            }
        }
    }
}
