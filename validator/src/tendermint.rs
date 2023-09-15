use std::sync::Arc;

use byteorder::WriteBytesExt;
use futures::{
    future::{self, FutureExt},
    stream::{BoxStream, StreamExt},
};
use nimiq_account::BlockLogger;
use nimiq_block::{Block, MacroBlock, TendermintProof};
use nimiq_blockchain::{BlockProducer, Blockchain};
use nimiq_blockchain_interface::AbstractBlockchain;
use nimiq_collections::BitSet;
use nimiq_handel::{aggregation::Aggregation, identity::IdentityRegistry};
use nimiq_hash::{Blake2sHash, Blake2sHasher, Hash, Hasher, SerializeContent};
use nimiq_keys::Signature as SchnorrSignature;
use nimiq_primitives::{
    policy::Policy, slots_allocation::Validators, TendermintIdentifier, TendermintStep,
    TendermintVote,
};
use nimiq_serde::Serialize;
use nimiq_tendermint::{
    Proposal, ProposalError, ProposalMessage, Protocol, SignedProposalMessage, Step,
    TaggedAggregationMessage,
};
use nimiq_validator_network::{
    single_response_requester::SingleResponseRequester, ValidatorNetwork,
};
use parking_lot::RwLock;

use crate::{
    aggregation::{
        registry::ValidatorRegistry,
        tendermint::{
            contribution::{AggregateMessage, TendermintContribution},
            proposal::{Body, Header, RequestProposal},
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
        (msg, recipient): (nimiq_handel::update::LevelUpdate<Self::Contribution>, usize),
    ) -> futures::future::BoxFuture<'static, ()> {
        // wrap the level update in the AggregateMessage
        let aggregation = AggregateMessage(msg);
        // tag it
        let tagged_aggregation_message = TaggedAggregationMessage {
            tag: self.tag,
            aggregation,
        };
        // and create the update.
        let update_message = TendermintUpdate(tagged_aggregation_message, self.height);

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
    <TValidatorNetwork as ValidatorNetwork>::PubsubId: std::fmt::Debug + Unpin,
{
    const PROPOSAL_PREFIX: u8 = TendermintStep::Propose as u8;

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

    /// Hashes the proposal, while taking care to not include the PubsubId
    ///
    /// This hash is NOT suited to be signed for BLS Aggregated signatures for the macro blocks, as those need to include the pk_tree_root.
    /// See MacroBlock::zkp_hash for more details.
    fn hash_proposal(proposal_msg: &ProposalMessage<<Self as Protocol>::Proposal>) -> Vec<u8> {
        let mut h = Blake2sHasher::new();

        h.write_u8(Self::PROPOSAL_PREFIX)
            .expect("Must be able to write Prefix to hasher");
        proposal_msg
            .proposal
            .0
            .serialize_content::<_, Blake2sHash>(&mut h)
            .expect("Must be able to serialize content of the proposal to hasher");
        proposal_msg
            .round
            .serialize_to_writer(&mut h)
            .expect("Must be able to serialize content of the round to hasher ");
        proposal_msg
            .valid_round
            .serialize_to_writer(&mut h)
            .expect("Must be able to serialize content of the valid_round to hasher ");

        let mut v = vec![];
        h.finish()
            .serialize_to_writer(&mut v)
            .expect("Must be able to serialize the hash.");

        v
    }
}

impl<TValidatorNetwork: ValidatorNetwork + 'static> Protocol
    for TendermintProtocol<TValidatorNetwork>
where
    <TValidatorNetwork as ValidatorNetwork>::PubsubId: std::fmt::Debug + Unpin,
{
    type Decision = MacroBlock;
    type Proposal = Header<<TValidatorNetwork as ValidatorNetwork>::PubsubId>;
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

    fn is_proposer(&self, round: u32) -> bool {
        let blockchain = self.blockchain.read();

        // Get best block for preceding micro block.
        // The best block might change, thus the vrf is not stored in separation
        let vrf_seed = match blockchain.get_block_at(self.block_height - 1, false, None) {
            Ok(Block::Micro(block)) => block.header.seed,
            _ => panic!("Preceding block must be a micro block and it must be known."),
        };

        // Get the validator for this round.
        let proposer_slot = blockchain
            .get_proposer_at(self.block_height, round, vrf_seed.entropy(), None)
            .expect("Couldn't find slot owner!");

        // Check if the slot bands match.
        // TODO Instead of identifying the validator by its slot_band, we should identify it by its
        // address instead.
        proposer_slot.band == self.validator_slot_band
    }

    fn create_proposal(&self, round: u32) -> (ProposalMessage<Self::Proposal>, Self::Inherent) {
        let blockchain = self.blockchain.read();
        let time = blockchain.time.now();

        let block = self
            .block_producer
            .next_macro_block_proposal(&blockchain, time, round, vec![]);

        // Always `Some(…)` because the above function always sets it to `Some(…)`.
        let body = block.body.expect("produced blocks always have a body");

        // Return the block header and body as the proposal.
        (
            ProposalMessage {
                proposal: Header(block.header, None), // Created proposals do not have a PubSubId
                round,
                valid_round: None,
            },
            Body(body),
        )
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
                        .get_proposer_at(
                            block_height,
                            signed_proposal.round,
                            vrf_seed.entropy(),
                            None,
                        )
                        .expect("Couldn't find slot owner!")
                        .validator
                        .signing_key;

                    let msg = signed_proposal.into_tendermint_signed_message(None);

                    let data = Self::hash_proposal(&msg.message);

                    if proposer.verify(&msg.signature.0, data.as_slice()) {
                        return Some(msg);
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
        signature_only: bool,
    ) -> Result<Self::Inherent, ProposalError> {
        // No inherent was given, but signature verification is the only required step, independent from
        // the result this can never work, as the inherent would have to be returned.
        if precalculated_inherent.is_none() && signature_only {
            return Err(ProposalError::Other);
        }

        // Abort if the blockchain is not in the correct state anymore.
        let blockchain = self.blockchain.read();
        if !blockchain.accounts_complete() {
            return Err(ProposalError::Other);
        }

        // Get best block for preceding micro block.
        // The best block might change, thus the vrf is not stored in separation
        let vrf_seed =
            match blockchain.get_block(&proposal.message.proposal.0.parent_hash, false, None) {
                // Block is known, proceed to verify the producer.
                Ok(Block::Micro(block)) => block.header.seed,
                // Block is not known, Cannot verify the proposal
                _ => return Err(ProposalError::Other),
            };

        // Get the validator for this round.
        let mut proposer = blockchain
            .get_proposer_at(
                self.block_height,
                proposal.message.round,
                vrf_seed.entropy(),
                None,
            )
            .expect("Couldn't find slot owner!")
            .validator
            .signing_key;

        // Verify the signature. The proposal is signed by the proposer of the round the proposal is used in.
        let proposal_hash = Self::hash_proposal(&proposal.message);
        if !proposer.verify(&proposal.signature.0, proposal_hash.as_slice()) {
            return Err(ProposalError::InvalidSignature);
        }

        // If only the signature verification is relevant, return immediately.
        if signature_only {
            // safe because signature_only && precalculated_inherent.is_none() was checked in the beginning of this function
            return Ok(precalculated_inherent.unwrap());
        }

        // Construct the block from the proposal.
        // We add the block body later once the proposal has been verified.
        let mut block = Block::Macro(MacroBlock {
            header: proposal.message.proposal.0.clone(),
            body: None,
            justification: None,
        });

        // If a valid round is set, the VRF seed will be signed by the proposer of that round.
        // So get that proposer.
        if let Some(vr) = proposal.message.valid_round {
            proposer = blockchain
                .get_proposer_at(self.block_height, vr, vrf_seed.entropy(), None)
                .expect("Couldn't find slot owner!")
                .validator
                .signing_key;
        }

        // Fetch predecessor block. Fail if it doesn't exist.
        let predecessor = blockchain
            .get_chain_info(block.parent_hash(), false, None)
            .map(|info| info.head)
            .map_err(|_| ProposalError::Other)?;

        if let Err(error) = block.header().verify(false) {
            debug!(%error, %block, "Tendermint - await_proposal: Invalid block header");
            Err(ProposalError::InvalidProposal)
        } else if let Err(error) = block.verify_immediate_successor(&predecessor) {
            debug!(%error, %block, "Tendermint - await_proposal: Invalid block header for blockchain head");
            Err(ProposalError::InvalidProposal)
        } else if let Err(error) = block.verify_macro_successor(&blockchain.macro_head()) {
            debug!(%error, %block, "Tendermint - await_proposal: Invalid block header for blockchain macro head");
            Err(ProposalError::InvalidProposal)
        } else if let Err(error) = block.verify_proposer(&proposer, predecessor.seed()) {
            debug!(%error, %block, "Tendermint - await_proposal: Invalid block header, VRF seed verification failed");
            Err(ProposalError::InvalidProposal)
        } else {
            // Get the blockchain state.
            let state = blockchain.state();

            // Compute block body if it is not cached.
            let macro_block = block.unwrap_macro_ref_mut();
            let body = match precalculated_inherent {
                Some(body) => body.0,
                None => BlockProducer::next_macro_body(&blockchain, &macro_block.header),
            };
            macro_block.body = Some(body);

            // Get a write transaction to the database, even if we don't intend to actually write
            // anything out.
            let mut txn = blockchain.write_transaction();

            // Verify macro block state before committing accounts.
            if let Err(error) = blockchain.verify_block_state_pre_commit(state, &block, &txn) {
                debug!(%error, %block, "Tendermint - await_proposal: Invalid macro block state");
                return Err(ProposalError::InvalidProposal);
            }

            // Update our blockchain state using the received proposal. If we can't update the state, we
            // return a proposal timeout.
            if let Err(error) = blockchain.commit_accounts(
                state,
                &block,
                None,
                &mut (&mut txn).into(),
                &mut BlockLogger::empty(),
            ) {
                debug!(%error, %block, "Tendermint - await_proposal: Failed to commit accounts");
                return Err(ProposalError::InvalidProposal);
            }

            // Check the validity of the block against our state. If it is invalid, we return a proposal
            // timeout. This also returns the block body that matches the block header
            // (assuming that the block is valid).
            if let Err(error) = blockchain.verify_block_state_post_commit(state, &block, &txn) {
                log::debug!(%error, %block, "Tendermint - await_proposal: Invalid block state");
                return Err(ProposalError::InvalidProposal);
            }

            // Return the body of the macro block so it can be cached.
            Ok(Body(block.unwrap_macro().body.unwrap()))
        }
    }

    fn sign_proposal(
        &self,
        proposal_message: &ProposalMessage<Self::Proposal>,
    ) -> Self::ProposalSignature {
        let data = Self::hash_proposal(proposal_message);
        (
            self.block_producer.signing_key.sign(data.as_slice()),
            self.validator_slot_band,
        )
    }

    fn create_aggregation(
        &self,
        round: u32,
        step: nimiq_tendermint::Step,
        proposal_hash: Option<Self::ProposalHash>,
        update_stream: BoxStream<'static, Self::AggregationMessage>,
    ) -> futures::stream::BoxStream<'static, Self::Aggregation> {
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
