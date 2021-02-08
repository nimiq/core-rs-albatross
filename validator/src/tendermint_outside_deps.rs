use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures::{stream::BoxStream, StreamExt};

use block_albatross::{
    Block, BlockHeader, MacroBlock, MacroBody, MacroHeader, MultiSignature,
    SignedTendermintProposal, TendermintProof, TendermintProposal,
};
use block_production_albatross::BlockProducer;
use blockchain_albatross::Blockchain;
use bls::{KeyPair, PublicKey};
use database::WriteTransaction;
use hash::{Blake2bHash, Hash};
use network_interface::network::Topic;
use nimiq_primitives::slot::ValidatorSlots;
use nimiq_validator_network::ValidatorNetwork;
use primitives::policy::{TENDERMINT_TIMEOUT_DELTA, TENDERMINT_TIMEOUT_INIT};
use primitives::slot::SlotCollection;
use tendermint::{
    AggregationResult, ProposalResult, Step, TendermintError, TendermintOutsideDeps,
    TendermintState,
};
use utils::time::OffsetTime;

use crate::aggregation::tendermint::HandelTendermintAdapter;

// TODO create stream immediately

struct ProposalTopic;
impl Topic for ProposalTopic {
    type Item = SignedTendermintProposal;

    fn topic(&self) -> String {
        "tendermint-proposal".to_owned()
    }

    fn validate(&self) -> bool {
        false
    }
}

/// The struct that interfaces with the Tendermint crate. It only has to implement the
/// TendermintOutsideDeps trait in order to do this.
pub struct TendermintInterface<N: ValidatorNetwork> {
    // The network that is going to be used to communicate with the other validators.
    pub network: Arc<N>,
    // This is used to maintain a network-wide time.
    pub offset_time: OffsetTime,
    // Necessary to produce blocks.
    pub block_producer: BlockProducer,
    // The main blockchain struct. Contains all of this validator information about the current chain.
    pub blockchain: Arc<Blockchain>,
    // The aggregation adapter allows Tendermint to use Handel functions and networking.
    pub aggregation_adapter: HandelTendermintAdapter<N>,
    // This validator's key pair.
    pub validator_key: KeyPair,
    // Just a field to temporarily store a block body. Since the body of a macro block is completely
    // deterministic, our Tendermint proposal only contains the block header. If the validator needs
    // the body, it is supposed for him to calculate it from the header and his current state.
    // However, calculating the body is an expensive operation. To avoid having to calculate the
    // body several times, we can cache it here.
    pub cache_body: Option<MacroBody>,

    proposal_stream:
        Option<BoxStream<'static, (SignedTendermintProposal, <N as ValidatorNetwork>::PubsubId)>>,
}

#[async_trait]
impl<N: ValidatorNetwork + 'static> TendermintOutsideDeps for TendermintInterface<N> {
    type ProposalTy = MacroHeader;
    type ProofTy = MultiSignature;
    type ResultTy = MacroBlock;

    /// This function is meant to verify the validity of a TendermintState. However, this function
    /// is only used when Tendermint is starting from a saved state. There is no reasonable
    /// situation where anyone would need to edit the saved TendermintState, so there's no situation
    /// where the TendermintState feed into this function would be invalid (unless it gets corrupted
    /// in memory, but then we have bigger problems).
    /// So, we leave this function simply returning true and not doing any checks. Mostly likely we
    /// will get rid of it in the future.
    fn verify_state(&self, _state: &TendermintState<Self::ProposalTy, Self::ProofTy>) -> bool {
        true
    }

    /// States if it is our turn to be the Tendermint proposer or not.
    fn is_our_turn(&self, round: u32) -> bool {
        // Get the validator slot for this round.
        let (slot, _) =
            self.blockchain
                .get_slot_owner_at(self.blockchain.block_number() + 1, round, None);

        // Get our public key.
        let our_public_key = self.validator_key.public_key.compress();

        // Compare the two public keys.
        slot.public_key().compressed() == &our_public_key
    }

    /// Produces a proposal. Evidently, used when we are the proposer.
    fn get_value(&mut self, round: u32) -> Result<Self::ProposalTy, TendermintError> {
        // Call the block producer to produce the next macro block (minus the justification, of course).
        let block =
            self.block_producer
                .next_macro_block_proposal(self.offset_time.now(), round, vec![]);

        // Cache the block body for future use.
        self.cache_body = block.body;

        // Return the block header as the proposal.
        Ok(block.header)
    }

    /// Assembles a block from a proposal and a proof.
    fn assemble_block(
        &self,
        round: u32,
        proposal: Self::ProposalTy,
        proof: Self::ProofTy,
    ) -> Result<Self::ResultTy, TendermintError> {
        // Get the body from our cache.
        let body = self.cache_body.clone();

        // Check that we have the correct body for our header.
        match &body {
            Some(body) => {
                if body.hash::<Blake2bHash>() != proposal.body_root {
                    debug!("Tendermint - assemble_block: Header and cached body don't match");
                    return Err(TendermintError::CannotAssembleBlock);
                }
            }
            None => {
                debug!("Tendermint - assemble_block: Cached body is None");
                return Err(TendermintError::CannotAssembleBlock);
            }
        }

        // Assemble the block and return it.
        Ok(MacroBlock {
            header: proposal,
            body,
            justification: Some(TendermintProof { round, sig: proof }),
        })
    }

    /// Broadcasts our proposal to the other validators.
    // Note: There might be situations when we broadcast the proposal before any other validator is
    // listening (for example, if we were also the producer of the last micro block before this
    // macro block). In that case, we will lose a Tendermint round unnecessarily. If this happens
    // frequently, it might make sense for us to have the validator broadcast his proposal twice.
    // One at the beginning and another at half of the timeout duration.
    async fn broadcast_proposal(
        &mut self,
        _round: u32,
        proposal: Self::ProposalTy,
        valid_round: Option<u32>,
    ) -> Result<(), TendermintError> {
        // Get the subscription stream from the network and store it if necessary
        // This needs to be done here, as publishing on a topic is only allowed when also subscribed to it.
        if self.proposal_stream.is_none() {
            let stream_result = self.network.subscribe(&ProposalTopic).await;
            if let Err(err) = stream_result {
                panic!("Could not open proposal stream: {:?}", err);
            } else {
                self.proposal_stream = stream_result.ok();
            }
        }

        // Get our validator index.
        let (validator_index, _) = self
            .blockchain
            .current_validators()
            .find_idx_and_num_slots_by_public_key(&self.validator_key.public_key.compress())
            .ok_or(TendermintError::ProposalBroadcastError)?;

        // Create the Tendermint proposal message.
        let proposal_message = TendermintProposal {
            value: proposal,
            valid_round,
        };

        // Sign the message with our validator key.
        let signed_proposal = SignedTendermintProposal::from_message(
            proposal_message,
            &self.validator_key.secret_key,
            validator_index,
        );

        // Broadcast the signed proposal to the network.
        if let Err(err) = self.network.publish(&ProposalTopic, signed_proposal).await {
            error!("Publishing proposal failed: {:?}", err);
        }

        Ok(())
    }

    /// Receives a proposal from this round's proposer. It also checks if the proposal is valid. If
    /// it doesn't a valid proposal from this round's proposer within a set time, then it returns
    /// Timeout.
    /// Note that it only accepts the first proposal sent by the proposer, valid or invalid. If it is
    /// invalid, then it will immediately return Timeout, even if the timeout duration hasn't elapsed
    /// yet.
    async fn await_proposal(
        &mut self,
        round: u32,
    ) -> Result<ProposalResult<Self::ProposalTy>, TendermintError> {
        // Get the proposer's slot and slot number for this round.
        let (slot, slot_number) = self.blockchain.get_slot_owner_at(
            self.blockchain.block_number() + 1,
            self.blockchain.view_number() + round,
            None,
        );

        // Calculate the validator id from the slot number.
        let validator_id = self
            .blockchain
            .current_validators()
            .get_band_number_by_slot_number(slot_number)
            .ok_or(TendermintError::CannotReceiveProposal)?;

        // Get the validator key.
        let validator_key = *slot.public_key().uncompress_unchecked();

        // Calculate the timeout duration.
        let timeout = Duration::from_millis(
            TENDERMINT_TIMEOUT_INIT + round as u64 * TENDERMINT_TIMEOUT_DELTA,
        );

        // This waits for a proposal from the proposer until it timeouts.
        let await_res = tokio::time::timeout(
            timeout,
            self.await_proposal_loop(validator_id, &validator_key),
        )
        .await;

        // Unwrap our await result. If we timed out, we return a proposal timeout right here.
        let proposal = match await_res {
            Ok(v) => v,
            Err(err) => {
                debug!("Tendermint - await_proposal: Timed out: {:?}", err);
                return Ok(ProposalResult::Timeout);
            }
        };

        // Get the header and valid round from the proposal.
        let header = proposal.value;
        let valid_round = proposal.valid_round;

        // Check the validity of the block header. If it is invalid, we return a proposal timeout
        // right here. This doesn't check anything that depends on the blockchain state.
        if self
            .blockchain
            .verify_block_header(&BlockHeader::Macro(header.clone()), &validator_key, None)
            .is_err()
        {
            debug!("Tendermint - await_proposal: Invalid block header");
            return Ok(ProposalResult::Timeout);
        }

        // Get a write transaction to the database.
        let mut txn = WriteTransaction::new(&self.blockchain.env);

        // Get the blockchain state.
        let state = self.blockchain.state();

        // Create a block with just our header.
        let block = Block::Macro(MacroBlock {
            header: header.clone(),
            body: None,
            justification: None,
        });

        let view_number = self.blockchain.head().next_view_number();

        // Update our blockchain state using the received proposal. If we can't update the state, we
        // return a proposal timeout right here.
        if self
            .blockchain
            .commit_accounts(&state, &block, 0, &mut txn) // view_number?
            .is_err()
        {
            debug!("Tendermint - await_proposal: Can't update state");
            return Ok(ProposalResult::Timeout);
        }

        // Check the validity of the block against our state. If it is invalid, we return a proposal
        // timeout right here. This also returns the block body that matches the block header
        // (assuming that the block is valid).
        let body = match self.blockchain.verify_block_state(&state, &block, Some(&mut txn)) {
            Ok(v) => v,
            Err(err) => {
                debug!("Tendermint - await_proposal: Invalid block state: {:?}", err);
                return Ok(ProposalResult::Timeout);
            }
        };

        // Cache the body that we calculated.
        self.cache_body = body;

        // Abort the transaction so that we don't commit the changes we made to the blockchain state.
        txn.abort();

        // Return the proposal.
        Ok(ProposalResult::Proposal(header, valid_round))
    }

    /// This broadcasts our vote for a given proposal and aggregates the votes from the other
    /// validators. It simply calls the aggregation adapter, which does all the work.
    async fn broadcast_and_aggregate(
        &mut self,
        round: u32,
        step: Step,
        proposal: Option<Blake2bHash>,
    ) -> Result<AggregationResult<Self::ProofTy>, TendermintError> {
        self.aggregation_adapter
            .broadcast_and_aggregate(round, step, proposal)
            .await
    }

    /// Returns the vote aggregation for a given proposal and round. It simply calls the aggregation
    /// adapter, which does all the work.
    async fn get_aggregation(
        &mut self,
        round: u32,
        step: Step,
    ) -> Result<AggregationResult<Self::ProofTy>, TendermintError> {
        self.aggregation_adapter.get_aggregate(round, step).await
    }
}

impl<N: ValidatorNetwork + 'static> TendermintInterface<N> {
    /// This function waits in a loop until it gets a proposal message from a given validator with a
    /// valid signature. It is just a helper function for the await_proposal function in this file.
    async fn await_proposal_loop(
        &mut self,
        validator_id: u16,
        validator_key: &PublicKey,
    ) -> TendermintProposal {
        // Get the subscription stream from the network and store it if necessary
        if self.proposal_stream.is_none() {
            let stream_result = self.network.subscribe(&ProposalTopic).await;
            if let Err(err) = stream_result {
                panic!("Could not open proposal stream: {:?}", err);
            } else {
                self.proposal_stream = stream_result.ok();
            }
        }

        while let Some((msg, _)) = self.proposal_stream.as_mut().unwrap().next().await {
            // Check if the proposal comes from the correct validator and the signature of the
            // proposal is valid. If not, keep awaiting.
            trace!("Received Proposal from {}", &msg.signer_idx);
            if validator_id == msg.signer_idx {
                if msg.verify(&validator_key) {
                    return msg.message;
                } else {
                    debug!("Tendermint - await_proposal: Invalid signature");
                }
            } else {
                debug!("Tendermint - await_proposal: Invalid validator id. Expected {}, found {}", validator_id, msg.signer_idx);
            }
        }

        // Evidently, the only way to escape the loop is to receive a valid message. But we need to
        // tell the Rust compiler this.
        unreachable!()
    }

    pub fn new(
        validator_key: KeyPair,
        validator_id: u16,
        network: Arc<N>,
        active_validators: ValidatorSlots,
        blockchain: Arc<Blockchain>,
        block_producer: BlockProducer,
        block_height: u32,
    ) -> Self {
        // Create the aggregation object.
        let aggregation_adapter = HandelTendermintAdapter::new(
            validator_id,
            active_validators,
            block_height,
            network.clone(),
            validator_key.secret_key,
        );

        // Create the instance and return it.
        Self {
            validator_key,
            network,
            aggregation_adapter,
            cache_body: None,
            block_producer,
            blockchain,
            offset_time: OffsetTime::default(),
            proposal_stream: None,
        }
    }
}
