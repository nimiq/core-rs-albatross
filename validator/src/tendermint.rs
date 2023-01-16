use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use beserial::Serialize;
use futures::{future::BoxFuture, stream::BoxStream, FutureExt, StreamExt};
use parking_lot::RwLock;

use nimiq_block::{
    Block, MacroBlock, MacroBody, MacroHeader, MultiSignature, ProposalTopic,
    SignedTendermintProposal, TendermintProof, TendermintProposal,
};
use nimiq_block_production::BlockProducer;
use nimiq_blockchain::Blockchain;
use nimiq_blockchain_interface::AbstractBlockchain;
use nimiq_bls::PublicKey;
use nimiq_hash::{Blake2bHash, Blake2sHash, Hash};
use nimiq_network_interface::network::MsgAcceptance;
use nimiq_primitives::{policy::Policy, slots::Validators};
use nimiq_tendermint::{
    AggregationResult, ProposalResult, Step, TendermintError, TendermintOutsideDeps,
    TendermintState,
};
use nimiq_utils::time::OffsetTime;
use nimiq_validator_network::ValidatorNetwork;
use nimiq_vrf::VrfSeed;

use crate::aggregation::tendermint::HandelTendermintAdapter;

/// The struct that interfaces with the Tendermint crate. It only has to implement the
/// TendermintOutsideDeps trait in order to do this.
pub struct TendermintInterface<TValidatorNetwork: ValidatorNetwork> {
    // The network that is going to be used to communicate with the other validators.
    pub network: Arc<TValidatorNetwork>,
    // This is used to maintain a network-wide time.
    pub offset_time: OffsetTime,
    // The slot band for our validator.
    pub validator_slot_band: u16,
    // The VRF seed of the parent block.
    pub prev_seed: VrfSeed,
    // The block number of the macro block to produce.
    pub block_height: u32,
    // Information relative to our validator that is necessary to produce blocks.
    pub block_producer: BlockProducer,
    // The validators for the current epoch.
    pub current_validators: Validators,
    // The main blockchain struct. Contains all of this validator information about the current chain.
    pub blockchain: Arc<RwLock<Blockchain>>,
    // The aggregation adapter allows Tendermint to use Handel functions and networking.
    pub aggregation_adapter: HandelTendermintAdapter<TValidatorNetwork>,

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
    type ProposalCacheTy = MacroBody;
    type ProposalHashTy = Blake2sHash;
    type ProofTy = MultiSignature;
    type ResultTy = MacroBlock;

    fn block_height(&self) -> u32 {
        self.block_height
    }

    fn initial_round(&self) -> u32 {
        // Macro blocks follow the same rules as micro blocks when it comes to round.
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
    fn verify_state(
        &self,
        state: &TendermintState<
            Self::ProposalTy,
            Self::ProposalCacheTy,
            Self::ProposalHashTy,
            Self::ProofTy,
        >,
    ) -> bool {
        state.height == self.block_height
    }

    /// States if it is our turn to be the Tendermint proposer or not.
    fn is_our_turn(&self, round: u32) -> bool {
        let blockchain = self.blockchain.read();

        // Get the validator for this round.
        let proposer_slot = blockchain
            .get_proposer_at(self.block_height, round, self.prev_seed.entropy(), None)
            .expect("Couldn't find slot owner!");

        // Check if the slot bands match.
        // TODO Instead of identifying the validator by its slot_band, we should identify it by its
        //  address instead.
        proposer_slot.band == self.validator_slot_band
    }

    /// Produces a proposal. Evidently, used when we are the proposer.
    fn get_value(
        &mut self,
        round: u32,
    ) -> Result<(Self::ProposalTy, Self::ProposalCacheTy), TendermintError> {
        let blockchain = self.blockchain.read();

        // Call the block producer to produce the next macro block (minus the justification, of course).
        let block = self.block_producer.next_macro_block_proposal(
            &blockchain,
            self.offset_time.now(),
            round,
            vec![],
        );

        // Always `Some(…)` because the above function always sets it to `Some(…)`.
        let body = block.body.expect("produced blocks always have a body");

        // Return the block header and body as the proposal.
        Ok((block.header, body))
    }

    /// Assembles a block from a proposal and a proof.
    fn assemble_block(
        &self,
        round: u32,
        proposal: Self::ProposalTy,
        proposal_cache: Option<Self::ProposalCacheTy>,
        proof: Self::ProofTy,
    ) -> Result<Self::ResultTy, TendermintError> {
        // Get the body from the cache. If there is no body cached it is recreated as a fallback.
        // However that operation is rather expensive annd should be avoided.
        let body = proposal_cache.or_else(|| {
            // Even though this fallback exist, it is unperformant to not cache the body.
            // Print a warning instead of failing.
            log::warn!("MacroBody was not cached. Recreating the body.");

            // Aquire a blockchain read lock
            let blockchain = self.blockchain.read();

            // The staking contract is holding the relevant informations
            let staking_contract = blockchain.get_staking_contract();

            // Compute the data for the MacroBody
            let lost_reward_set = staking_contract.previous_lost_rewards();
            let disabled_set = staking_contract.previous_disabled_slots();
            let validators = if Policy::is_election_block_at(proposal.block_number) {
                Some(blockchain.next_validators(&proposal.seed))
            } else {
                None
            };
            let pk_tree_root = validators
                .as_ref()
                .and_then(|validators| MacroBlock::pk_tree_root(validators).ok());

            // Assemble the MacroBody
            Some(MacroBody {
                validators,
                pk_tree_root,
                lost_reward_set,
                disabled_set,
            })
        });

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
        self,
        round: u32,
        proposal: Self::ProposalTy,
        valid_round: Option<u32>,
    ) -> Result<Self, TendermintError> {
        // Create the Tendermint proposal message.
        let proposal_message = TendermintProposal {
            value: proposal,
            valid_round,
            round,
        };

        // Sign the message with our validator key.
        let signed_proposal = SignedTendermintProposal::from_message(
            proposal_message,
            &self.block_producer.voting_key.secret_key,
            self.validator_slot_band,
        );

        debug!(
            round = round,
            valid_round = valid_round,
            "Broadcasting tendermint proposal"
        );

        // Broadcast the signed proposal to the network.
        if let Err(err) = self.network.publish::<ProposalTopic>(signed_proposal).await {
            error!("Publishing proposal failed: {:?}", err);
        }

        Ok(self)
    }

    /// Receives a proposal from this round's proposer. It also checks if the proposal is valid. If
    /// it doesn't a valid proposal from this round's proposer within a set time, then it returns
    /// Timeout.
    /// Note that it only accepts the first proposal sent by the proposer, valid or invalid. If it is
    /// invalid, then it will immediately return Timeout, even if the timeout duration hasn't elapsed
    /// yet.
    async fn await_proposal(
        mut self,
        round: u32,
    ) -> Result<
        (
            Self,
            ProposalResult<Self::ProposalTy, Self::ProposalCacheTy>,
        ),
        TendermintError,
    > {
        let (timeout, proposer_slot_band, proposer_voting_key, proposer_signing_key) = {
            let blockchain = self.blockchain.read();

            // Get the proposer's slot and slot number for this round.
            let proposer_slot = blockchain
                .get_proposer_at(self.block_height, round, self.prev_seed.entropy(), None)
                .expect("Couldn't find slot owner!");
            let proposer_slot_band = proposer_slot.band;

            // Get the validator keys.
            let proposer_voting_key = *proposer_slot.validator.voting_key.uncompress_unchecked();
            let proposer_signing_key = proposer_slot.validator.signing_key;

            // Calculate the timeout duration.
            let timeout = Duration::from_millis(
                Policy::tendermint_timeout_init()
                    + round as u64 * Policy::tendermint_timeout_delta(),
            );

            debug!(
                block_number = blockchain.block_number() + 1,
                round = &round,
                slot_band = &proposer_slot_band,
                "Awaiting proposal timeout={:?}",
                &timeout
            );

            (
                timeout,
                proposer_slot_band,
                proposer_voting_key,
                proposer_signing_key,
            )
        };

        // This waits for a proposal from the proposer until it timeouts.
        let await_res = tokio::time::timeout(
            timeout,
            self.await_proposal_loop(
                proposer_slot_band,
                &proposer_voting_key,
                self.block_height,
                round,
            ),
        )
        .await;

        // Unwrap our await result. If we timed out, we return a proposal timeout right here.
        let (proposal, id) = match await_res {
            Ok(v) => v,
            Err(err) => {
                debug!("Tendermint - await_proposal: Timed out: {:?}", err);
                return Ok((self, ProposalResult::Timeout));
            }
        };

        let (acceptance, valid_round, header, body) = {
            let blockchain = self.blockchain.read();

            // Get the header and valid round from the proposal.
            let header = proposal.value;
            let valid_round = proposal.valid_round;

            // In case the proposal has a valid round, the original proposer signed the VRF Seed,
            // so the original slot owners key must be retrieved for header verification.
            let proposer_key = if valid_round.is_some() {
                let proposer_slot = blockchain
                    .get_proposer_at(
                        self.block_height,
                        header.round,
                        self.prev_seed.entropy(),
                        None,
                    )
                    .expect("Couldn't find slot owner!");

                proposer_slot.validator.signing_key
            } else {
                proposer_signing_key
            };

            // Create a block with just our header.
            let block = Block::Macro(MacroBlock {
                header: header.clone(),
                body: None,
                justification: None,
            });

            // Check the validity of the block header. If it is invalid, we return a proposal timeout
            // right here. This doesn't check anything that depends on the blockchain state.
            let head = blockchain.head();
            if let Err(error) = block.header().verify(false) {
                debug!(%error, "Tendermint - await_proposal: Invalid block header");
                (MsgAcceptance::Reject, valid_round, None, None)
            } else if let Err(error) = block.verify_immediate_successor(&head) {
                debug!(%error, "Tendermint - await_proposal: Invalid block header for blockchain head");
                (MsgAcceptance::Reject, valid_round, None, None)
            } else if let Err(error) = block.verify_macro_successor(&blockchain.macro_head()) {
                debug!(%error, "Tendermint - await_proposal: Invalid block header for blockchain macro head");
                (MsgAcceptance::Reject, valid_round, None, None)
            } else if let Err(error) = block.verify_proposer(&proposer_key, head.seed()) {
                debug!(%error, "Tendermint - await_proposal: Invalid block header, VRF seed verification failed");
                (MsgAcceptance::Reject, valid_round, None, None)
            } else {
                // Get a write transaction to the database.
                let mut txn = blockchain.write_transaction();

                // Get the blockchain state.
                let state = blockchain.state();

                // Update our blockchain state using the received proposal. If we can't update the state, we
                // return a proposal timeout.
                if blockchain.commit_accounts(state, &block, &mut txn).is_err() {
                    debug!("Tendermint - await_proposal: Can't update state");
                    (MsgAcceptance::Reject, valid_round, None, None)
                } else {
                    // Check the validity of the block against our state. If it is invalid, we return a proposal
                    // timeout. This also returns the block body that matches the block header
                    // (assuming that the block is valid).
                    let block_state = blockchain.verify_block_state(state, &block, Some(&txn));

                    match block_state {
                        Ok(body) => (
                            MsgAcceptance::Accept,
                            valid_round,
                            Some(header),
                            Some(body.expect(
                                "verify_block_state returns a body for blocks without one",
                            )),
                        ),
                        Err(err) => {
                            debug!(
                                "Tendermint - await_proposal: Invalid block state: {:?}",
                                err
                            );
                            (MsgAcceptance::Reject, valid_round, None, None)
                        }
                    }
                }
            }
        };

        // Indicate the messages acceptance to the network
        self.network
            .validate_message::<ProposalTopic>(id, acceptance);

        // Regardless of broadcast result, process proposal if it exists. Timeout otherwise.
        if let Some(header) = header {
            // Return the proposal.
            Ok((
                self,
                ProposalResult::Proposal((header, body.expect("Body must exist")), valid_round),
            ))
        } else {
            Ok((self, ProposalResult::Timeout))
        }
    }

    /// This broadcasts our vote for a given proposal and aggregates the votes from the other
    /// validators. It simply calls the aggregation adapter, which does all the work.
    async fn broadcast_and_aggregate(
        mut self,
        round: u32,
        step: Step,
        proposal_hash: Option<Self::ProposalHashTy>,
    ) -> Result<(Self, AggregationResult<Self::ProposalHashTy, Self::ProofTy>), TendermintError>
    {
        self.aggregation_adapter
            .broadcast_and_aggregate(round, step, proposal_hash)
            .await
            .map(|result| (self, result))
    }

    fn rebroadcast_and_aggregate(
        &self,
        round: u32,
        step: Step,
        proposal_hash: Option<Self::ProposalHashTy>,
    ) {
        self.aggregation_adapter
            .rebroadcast_and_aggregate(round, step, proposal_hash)
    }

    /// Returns the vote aggregation for a given round and step. It simply calls the aggregation
    /// adapter, which does all the work.
    async fn get_aggregation(
        self,
        round: u32,
        step: Step,
    ) -> Result<(Self, AggregationResult<Self::ProposalHashTy, Self::ProofTy>), TendermintError>
    {
        self.aggregation_adapter
            .get_aggregate(round, step)
            .map(|result| (self, result))
    }

    /// Calculates the nano_zkp_hash used as the proposal hash, but for performance reasons we fetch
    /// the pk_tree_root from the already cached block body.
    fn hash_proposal(
        &self,
        proposal: Self::ProposalTy,
        proposal_cache: Self::ProposalCacheTy,
    ) -> Self::ProposalHashTy {
        // Calculate the header hash.
        let mut message = proposal.hash::<Blake2bHash>().serialize_to_vec();

        // Fetch the pk_tree_root.
        let pk_tree_root = proposal_cache.pk_tree_root;

        // If it is Some, add its contents to the message.
        if let Some(mut bytes) = pk_tree_root {
            message.append(&mut bytes);
        }

        // Return the final hash.
        message.hash::<Blake2sHash>()
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
        validator_slot_band: u16,
        validator_key: &PublicKey,
        expected_height: u32,
        expected_round: u32,
    ) -> (TendermintProposal, TValidatorNetwork::PubsubId) {
        while let Some((msg, id)) = self.proposal_stream.as_mut().next().await {
            // most basic check first: only process current height proposals, discard old ones
            if msg.message.value.block_number == expected_height
                && msg.message.round == expected_round
            {
                // Check if the proposal comes from the correct validator and the signature of the
                // proposal is valid. If not, keep awaiting.
                debug!(
                    block_number = &msg.message.value.block_number,
                    round = &msg.message.round,
                    validator_idx = &msg.signer_idx,
                    "Received Tendermint Proposal"
                );

                if validator_slot_band == msg.signer_idx {
                    if msg.verify(validator_key) {
                        return (msg.message, id);
                    } else {
                        debug!("Tendermint - await_proposal: Invalid signature");
                    }
                } else {
                    debug!(
                        "Tendermint - await_proposal: Invalid validator id. Expected {}, found {}",
                        validator_slot_band, msg.signer_idx
                    );
                }
            }
        }

        // Evidently, the only way to escape the loop is to receive a valid message. But we need to
        // tell the Rust compiler this.
        unreachable!()
    }

    pub fn new(
        validator_slot_band: u16,
        active_validators: Validators,
        prev_seed: VrfSeed,
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
            validator_slot_band,
            active_validators.clone(),
            block_height,
            network.clone(),
            block_producer.voting_key.secret_key,
        );

        // Create the instance and return it.
        Self {
            network,
            offset_time: OffsetTime::default(),
            validator_slot_band,
            prev_seed,
            block_height,
            block_producer,
            current_validators: active_validators,
            blockchain,
            aggregation_adapter,
            proposal_stream,
            initial_round,
        }
    }
}
