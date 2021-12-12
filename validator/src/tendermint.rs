use std::ops::Deref;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures::{
    future::{BoxFuture, FutureExt},
    stream::{BoxStream, StreamExt},
};
use parking_lot::RwLock;

use block::{
    Block, BlockHeader, MacroBlock, MacroBody, MacroHeader, MultiSignature,
    SignedTendermintProposal, TendermintProof, TendermintProposal,
};
use block_production::BlockProducer;
use blockchain::{AbstractBlockchain, Blockchain};
use bls::{KeyPair, PublicKey};
use hash::{Blake2bHash, Blake2sHash, Hash};
use nimiq_network_interface::network::MsgAcceptance;
use nimiq_validator_network::ValidatorNetwork;
use primitives::{
    policy::{TENDERMINT_TIMEOUT_DELTA, TENDERMINT_TIMEOUT_INIT},
    slots::Validators,
};
use tendermint_protocol::{
    AggregationResult, ProposalResult, Step, TendermintError, TendermintOutsideDeps,
    TendermintState,
};
use utils::time::OffsetTime;

use crate::aggregation::tendermint::HandelTendermintAdapter;
use crate::validator::ProposalTopic;

/// The struct that interfaces with the Tendermint crate. It only has to implement the
/// TendermintOutsideDeps trait in order to do this.
pub struct TendermintInterface<TValidatorNetwork: ValidatorNetwork> {
    // The network that is going to be used to communicate with the other validators.
    pub network: Arc<TValidatorNetwork>,
    // This is used to maintain a network-wide time.
    pub offset_time: OffsetTime,
    // Necessary to produce blocks.
    pub block_producer: BlockProducer,
    // The main blockchain struct. Contains all of this validator information about the current chain.
    pub blockchain: Arc<RwLock<Blockchain>>,
    // The aggregation adapter allows Tendermint to use Handel functions and networking.
    pub aggregation_adapter: HandelTendermintAdapter<TValidatorNetwork>,
    // This is the validator's voting key pair.
    pub validator_key: KeyPair,
    // Just a field to temporarily store a block body. Since the body of a macro block is completely
    // deterministic, our Tendermint proposal only contains the block header. If the validator needs
    // the body, it is supposed for him to calculate it from the header and his current state.
    // However, calculating the body is an expensive operation. To avoid having to calculate the
    // body several times, we can cache it here.
    pub cache_body: Option<MacroBody>,
    // Just like above, calculating the block hash (at least the `nano_zkp_hash`) is an expensive
    // operation. So we cache it here to avoid recalculation.
    pub cache_hash: Option<Blake2sHash>,

    proposal_stream: BoxStream<
        'static,
        (
            SignedTendermintProposal,
            <TValidatorNetwork as ValidatorNetwork>::PubsubId,
        ),
    >,

    initial_round: u32,
}

#[async_trait]
impl<TValidatorNetwork: ValidatorNetwork + 'static> TendermintOutsideDeps
    for TendermintInterface<TValidatorNetwork>
{
    type ProposalTy = MacroHeader;
    type ProposalHashTy = Blake2sHash;
    type ProofTy = MultiSignature;
    type ResultTy = MacroBlock;

    fn initial_round(&self) -> u32 {
        // Macro blocks follow the same rules as micro blocks when it comes to view_number/round.
        // Thus the round is offset by the predecessors view.
        self.initial_round
    }

    /// This function is meant to verify the validity of a TendermintState. However, this function
    /// is only used when Tendermint is starting from a saved state. There is no reasonable
    /// situation where anyone would need to edit the saved TendermintState, so there's no situation
    /// where the TendermintState fed into this function would be invalid (unless it gets corrupted
    /// in memory, but then we have bigger problems).
    /// So, we leave this function simply returning true and not doing any checks. Mostly likely we
    /// will get rid of it in the future.
    fn verify_state(&self, state: &TendermintState<Self::ProposalTy, Self::ProofTy>) -> bool {
        self.initial_round() <= state.round
    }

    /// States if it is our turn to be the Tendermint proposer or not.
    fn is_our_turn(&self, round: u32) -> bool {
        let blockchain = self.blockchain.read();
        // Get the validator slot for this round.
        let (slot, _) = blockchain
            .get_slot_owner_at(blockchain.block_number() + 1, round, None)
            .expect("Couldn't find slot owner!");

        // Get our public key.
        let our_public_key = self.validator_key.public_key.compress();

        // Compare the two public keys.
        slot.voting_key.compressed() == &our_public_key
    }

    /// Produces a proposal. Evidently, used when we are the proposer.
    fn get_value(&mut self, round: u32) -> Result<Self::ProposalTy, TendermintError> {
        let blockchain = self.blockchain.read();
        // Call the block producer to produce the next macro block (minus the justification, of course).
        let block = self.block_producer.next_macro_block_proposal(
            &blockchain,
            self.offset_time.now(),
            round,
            vec![],
        );

        // Cache the block body and hash for future use.
        self.cache_hash = Some(block.nano_zkp_hash());
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
        round: u32,
        proposal: Self::ProposalTy,
        valid_round: Option<u32>,
    ) -> Result<(), TendermintError> {
        // Get our validator index.
        // TODO: This code block gets this validators position in the validators struct by searching it
        //  with its public key. This is an insane way of doing this. Just start saving the validator
        //  id somewhere here.
        let mut validator_index_opt = None;
        for (i, validator) in self
            .blockchain
            .read()
            .current_validators()
            .unwrap()
            .iter()
            .enumerate()
        {
            if validator.voting_key.compressed() == &self.validator_key.public_key.compress() {
                validator_index_opt = Some(i as u16);
                break;
            }
        }
        let validator_index = validator_index_opt.ok_or(TendermintError::ProposalBroadcastError)?;

        // Create the Tendermint proposal message.
        let proposal_message = TendermintProposal {
            value: proposal,
            valid_round,
            round,
        };

        // Sign the message with our validator key.
        let signed_proposal = SignedTendermintProposal::from_message(
            proposal_message,
            &self.validator_key.secret_key,
            validator_index,
        );

        // Broadcast the signed proposal to the network.
        if let Err(err) = self.network.publish::<ProposalTopic>(signed_proposal).await {
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
        let (timeout, validator_id, voting_key, signing_key, expected_height) = {
            let blockchain = self.blockchain.read();

            // Get the blockchain height.
            let expected_height = blockchain.block_number() + 1;

            // Get the proposer's slot and slot number for this round.
            let (slot, slot_number) = blockchain
                .get_slot_owner_at(expected_height, round, None)
                .expect("Couldn't find slot owner!");

            // Calculate the validator slot band from the slot number.
            // TODO: Again, just redo this. We shouldn't be using slot bands. Validator ID is a much better
            //  field.
            let validator_id = blockchain
                .current_validators()
                .unwrap()
                .get_band_from_slot(slot_number);

            // Get the validator keys.
            let voting_key = *slot.voting_key.uncompress_unchecked();
            let signing_key = slot.signing_key;

            // Calculate the timeout duration.
            let timeout = Duration::from_millis(
                TENDERMINT_TIMEOUT_INIT + round as u64 * TENDERMINT_TIMEOUT_DELTA,
            );

            debug!(
                "Awaiting proposal for {}.{}, expected producer: {}, timeout: {:?}",
                blockchain.block_number() + 1,
                &round,
                &validator_id,
                &timeout
            );

            (
                timeout,
                validator_id,
                voting_key,
                signing_key,
                expected_height,
            )
        };

        // This waits for a proposal from the proposer until it timeouts.
        let await_res = tokio::time::timeout(
            timeout,
            self.await_proposal_loop(validator_id, &voting_key, expected_height, round),
        )
        .await;

        // Unwrap our await result. If we timed out, we return a proposal timeout right here.
        let (proposal, id) = match await_res {
            Ok(v) => v,
            Err(err) => {
                debug!("Tendermint - await_proposal: Timed out: {:?}", err);
                return Ok(ProposalResult::Timeout);
            }
        };

        let acceptance = {
            let blockchain = self.blockchain.read();

            // Get the header and valid round from the proposal.
            let header = proposal.value;
            let valid_round = proposal.valid_round;

            // In case the proposal has a valid round, the original proposer signed the VRF Seed,
            // so the original slot owners key must be retrieved for header verification.
            // View numbers in macro blocks denote the original proposers round.
            let vrf_key = if valid_round.is_some() {
                let (valid_round_slot, _) = blockchain
                    .get_slot_owner_at(expected_height, header.view_number, None)
                    .expect("Couldn't find slot owner!");

                valid_round_slot.signing_key
            } else {
                signing_key
            };

            // Check the validity of the block header. If it is invalid, we return a proposal timeout
            // right here. This doesn't check anything that depends on the blockchain state.
            if Blockchain::verify_block_header(
                blockchain.deref(),
                &BlockHeader::Macro(header.clone()),
                &vrf_key,
                None,
                true,
            )
            .is_err()
            {
                debug!("Tendermint - await_proposal: Invalid block header");
                None
            } else {
                let mut acceptance = MsgAcceptance::Accept;

                // Get a write transaction to the database.
                let mut txn = blockchain.write_transaction();

                // Get the blockchain state.
                let state = blockchain.state();

                // Create a block with just our header.
                let block = Block::Macro(MacroBlock {
                    header: header.clone(),
                    body: None,
                    justification: None,
                });

                // Update our blockchain state using the received proposal. If we can't update the state, we
                // return a proposal timeout.
                if blockchain
                    .commit_accounts(state, &block, 0, &mut txn)
                    .is_err()
                {
                    debug!("Tendermint - await_proposal: Can't update state");
                    acceptance = MsgAcceptance::Reject;
                } else {
                    // Check the validity of the block against our state. If it is invalid, we return a proposal
                    // timeout. This also returns the block body that matches the block header
                    // (assuming that the block is valid).
                    let block_state = blockchain.verify_block_state(state, &block, Some(&txn));

                    if let Ok(body) = block_state {
                        // Calculate and cache the block hash.
                        let macro_block = MacroBlock {
                            header: header.clone(),
                            body: body.clone(),
                            justification: None,
                        };

                        self.cache_hash = Some(macro_block.nano_zkp_hash());

                        // Cache the body that we calculated.
                        self.cache_body = body;
                    } else if let Err(err) = block_state {
                        debug!(
                            "Tendermint - await_proposal: Invalid block state: {:?}",
                            err
                        );
                        acceptance = MsgAcceptance::Reject;
                    }
                }

                // Abort the transaction so that we don't commit the changes we made to the blockchain state.
                txn.abort();

                Some((acceptance, header, valid_round))
            }
        };

        // If the message was validated successfully, the network may now relay it to other peers.
        // Otherwise, reject or ignore the message.
        if let Some((MsgAcceptance::Accept, header, valid_round)) = acceptance {
            self.network
                .validate_message(id, MsgAcceptance::Accept)
                .await
                .unwrap();

            // Return the proposal.
            Ok(ProposalResult::Proposal(header, valid_round))
        } else {
            self.network
                .validate_message(id, MsgAcceptance::Reject)
                .await
                .unwrap();
            Ok(ProposalResult::Timeout)
        }
    }

    /// This broadcasts our vote for a given proposal and aggregates the votes from the other
    /// validators. It simply calls the aggregation adapter, which does all the work.
    async fn broadcast_and_aggregate(
        &mut self,
        round: u32,
        step: Step,
        proposal_hash: Option<Self::ProposalHashTy>,
    ) -> Result<AggregationResult<Self::ProposalHashTy, Self::ProofTy>, TendermintError> {
        self.aggregation_adapter
            .broadcast_and_aggregate(round, step, proposal_hash)
            .await
    }

    /// Returns the vote aggregation for a given round and step. It simply calls the aggregation
    /// adapter, which does all the work.
    async fn get_aggregation(
        &mut self,
        round: u32,
        step: Step,
    ) -> Result<AggregationResult<Self::ProposalHashTy, Self::ProofTy>, TendermintError> {
        self.aggregation_adapter.get_aggregate(round, step)
    }

    /// Simply fetches the cached proposal hash.
    fn hash_proposal(&self, _proposal: Self::ProposalTy) -> Self::ProposalHashTy {
        self.cache_hash
            .clone()
            .expect("Tried to fetch a non-existing proposal hash. This shouldn't happen!")
    }

    fn get_background_task(&mut self) -> BoxFuture<'static, ()> {
        self.aggregation_adapter.create_background_task().boxed()
    }
}

impl<TValidatorNetwork: ValidatorNetwork + 'static> TendermintInterface<TValidatorNetwork> {
    /// This function waits in a loop until it gets a proposal message from a given validator with a
    /// valid signature. It is just a helper function for the await_proposal function in this file.
    async fn await_proposal_loop(
        &mut self,
        validator_id: u16,
        validator_key: &PublicKey,
        expected_height: u32,
        expected_round: u32,
    ) -> (TendermintProposal, TValidatorNetwork::PubsubId) {
        while let Some((msg, id)) = self.proposal_stream.as_mut().next().await {
            // most basic check first: only process current height proposals, discard old ones
            if msg.message.value.block_number == expected_height
                && msg.message.round == expected_round
            {
                // view number
                // Check if the proposal comes from the correct validator and the signature of the
                // proposal is valid. If not, keep awaiting.
                debug!(
                    "Received Proposal for block #{}.{} from validator {} ",
                    &msg.message.value.block_number, &msg.message.round, &msg.signer_idx,
                );
                if validator_id == msg.signer_idx {
                    if msg.verify(validator_key) {
                        return (msg.message, id);
                    } else {
                        debug!("Tendermint - await_proposal: Invalid signature");
                    }
                } else {
                    debug!(
                        "Tendermint - await_proposal: Invalid validator id. Expected {}, found {}",
                        validator_id, msg.signer_idx
                    );
                }
            }
        }

        // Evidently, the only way to escape the loop is to receive a valid message. But we need to
        // tell the Rust compiler this.
        unreachable!()
    }

    pub fn new(
        voting_key: KeyPair,
        validator_id: u16,
        active_validators: Validators,
        block_height: u32,
        network: Arc<TValidatorNetwork>,
        blockchain: Arc<RwLock<Blockchain>>,
        block_producer: BlockProducer,
        proposal_stream: BoxStream<
            'static,
            (
                SignedTendermintProposal,
                <TValidatorNetwork as ValidatorNetwork>::PubsubId,
            ),
        >,
        initial_round: u32,
    ) -> Self {
        // Create the aggregation object.
        let aggregation_adapter = HandelTendermintAdapter::new(
            validator_id,
            active_validators,
            block_height,
            network.clone(),
            voting_key.secret_key,
        );

        // Create the instance and return it.
        Self {
            network,
            offset_time: OffsetTime::default(),
            block_producer,
            blockchain,
            aggregation_adapter,
            validator_key: voting_key,
            cache_body: None,
            cache_hash: None,
            proposal_stream,
            initial_round,
        }
    }
}
