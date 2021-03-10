use std::io;
use std::pin::Pin;
use std::sync::Arc;

use futures::stream::{BoxStream, Stream, StreamExt};
use futures::task::{Context, Poll};

use beserial::{Deserialize, Serialize};
use nimiq_block_albatross::{
    MacroBlock, MacroHeader, MultiSignature, SignedTendermintProposal, TendermintStep,
};
use nimiq_block_production_albatross::BlockProducer;
use nimiq_blockchain_albatross::{AbstractBlockchain, Blockchain};
use nimiq_database::{FromDatabaseValue, IntoDatabaseValue};
use nimiq_tendermint::{
    Checkpoint, Step, TendermintOutsideDeps, TendermintReturn, TendermintState,
};
use nimiq_validator_network::ValidatorNetwork;

use crate::tendermint_outside_deps::TendermintInterface;

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
        blockchain: Arc<Blockchain>,
        network: Arc<TValidatorNetwork>,
        block_producer: BlockProducer,
        signing_key: bls::KeyPair, // probably SecretKey is enough (it is for the handel part of it).
        validator_id: u16,
        state: Option<PersistedMacroState<TValidatorNetwork>>,
        proposal_stream: BoxStream<
            'static,
            (
                SignedTendermintProposal,
                <TValidatorNetwork as ValidatorNetwork>::PubsubId,
            ),
        >,
    ) -> Self {
        // get validators for current epoch
        let active_validators = blockchain.current_validators().unwrap();

        // create the TendermintOutsideDeps instance
        // Replace here with the actual OutSide Deps instead of the Mocked ones.
        let deps = TendermintInterface::new(
            signing_key,
            validator_id,
            network,
            active_validators,
            blockchain.clone(),
            block_producer,
            blockchain.head().block_number() + 1,
            proposal_stream,
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

        // create the Tendermint instance which is done using the expect_block function
        let tendermint = Box::pin(nimiq_tendermint::expect_block(deps, state_opt));

        // Create the instance and return it.
        Self { tendermint }
    }
}

impl Stream for ProduceMacroBlock {
    type Item = TendermintReturn<MacroHeader, MultiSignature, MacroBlock>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.tendermint.poll_next_unpin(cx)
    }
}
