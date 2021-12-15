use std::io;
use std::pin::Pin;
use std::sync::Arc;

use futures::stream::{BoxStream, Stream, StreamExt};
use futures::task::{Context, Poll};
use parking_lot::RwLock;

use beserial::{Deserialize, Serialize};
use nimiq_block::{
    MacroBlock, MacroHeader, MultiSignature, SignedTendermintProposal, TendermintStep,
};
use nimiq_block_production::BlockProducer;
use nimiq_blockchain::Blockchain;
use nimiq_database::{FromDatabaseValue, IntoDatabaseValue};
use nimiq_primitives::slots::Validators;
use nimiq_tendermint::{
    Checkpoint, Step, TendermintOutsideDeps, TendermintReturn, TendermintState,
};
use nimiq_validator_network::ValidatorNetwork;

use crate::tendermint::TendermintInterface;

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct PersistedMacroState<TValidatorNetwork: ValidatorNetwork + 'static> {
    pub height: u32,
    pub round: u32,
    pub step: TendermintStep,
    pub locked_value:
        Option<<TendermintInterface<TValidatorNetwork> as TendermintOutsideDeps>::ProposalTy>,
    pub locked_round: Option<u32>,
    pub valid_value:
        Option<<TendermintInterface<TValidatorNetwork> as TendermintOutsideDeps>::ProposalTy>,
    pub valid_round: Option<u32>,
}

impl<TValidatorNetwork: ValidatorNetwork> IntoDatabaseValue
    for PersistedMacroState<TValidatorNetwork>
{
    fn database_byte_size(&self) -> usize {
        self.serialized_size()
    }

    fn copy_into_database(&self, mut bytes: &mut [u8]) {
        Serialize::serialize(&self, &mut bytes).unwrap();
    }
}

impl<TValidatorNetwork: ValidatorNetwork> FromDatabaseValue
    for PersistedMacroState<TValidatorNetwork>
{
    fn copy_from_database(bytes: &[u8]) -> io::Result<Self>
    where
        Self: Sized,
    {
        let mut cursor = io::Cursor::new(bytes);
        Ok(Deserialize::deserialize(&mut cursor)?)
    }
}

pub(crate) struct ProduceMacroBlock {
    tendermint: BoxStream<'static, TendermintReturn<MacroHeader, MultiSignature, MacroBlock>>,
}

impl ProduceMacroBlock {
    pub fn new<TValidatorNetwork: ValidatorNetwork + 'static>(
        blockchain: Arc<RwLock<Blockchain>>,
        network: Arc<TValidatorNetwork>,
        block_producer: BlockProducer,
        validator_slot_band: u16,
        active_validators: Validators,
        block_height: u32,
        initial_round: u32,
        state: Option<PersistedMacroState<TValidatorNetwork>>,
        proposal_stream: BoxStream<
            'static,
            (
                SignedTendermintProposal,
                <TValidatorNetwork as ValidatorNetwork>::PubsubId,
            ),
        >,
    ) -> Self {
        // create the TendermintOutsideDeps instance
        // Replace here with the actual OutSide Deps instead of the Mocked ones.
        let deps = TendermintInterface::new(
            validator_slot_band,
            active_validators,
            block_height,
            network,
            blockchain,
            block_producer,
            proposal_stream,
            initial_round,
        );

        let state_opt = state.map(|s| TendermintState {
            step: match s.step {
                TendermintStep::PreVote => Step::Prevote,
                TendermintStep::PreCommit => Step::Precommit,
                TendermintStep::Propose => Step::Propose,
            },
            round: s.round,
            locked_value: s.locked_value,
            locked_round: s.locked_round,
            valid_value: s.valid_value,
            valid_round: s.valid_round,
            current_checkpoint: Checkpoint::StartRound,
            current_proof: None,
            current_proposal: None,
            current_proposal_vr: None,
        });

        // create the Tendermint instance, which implements Stream

        let tendermint = match nimiq_tendermint::Tendermint::new(deps, state_opt) {
            Ok(tendermint) => tendermint,
            Err(returned_deps) => {
                log::debug!("TendermintState was invalid. Restarting Tendermint without state");
                // new only returns None if the state failed to verify, which without a state is impossible.
                // Thus unwrapping is safe.
                match nimiq_tendermint::Tendermint::new(returned_deps, None) {
                    Ok(tendermint) => tendermint,
                    Err(_) => unreachable!(),
                }
            }
        };

        // Create the instance and return it.
        Self {
            tendermint: Box::pin(tendermint),
        }
    }
}

impl Stream for ProduceMacroBlock {
    type Item = TendermintReturn<MacroHeader, MultiSignature, MacroBlock>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.tendermint.poll_next_unpin(cx)
    }
}
