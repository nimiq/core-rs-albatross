use std::io;
use std::pin::Pin;
use std::sync::Arc;

use futures::stream::{BoxStream, Stream, StreamExt};
use futures::task::{Context, Poll};
use parking_lot::RwLock;

use beserial::{Deserialize, Serialize};
use nimiq_block::SignedTendermintProposal;
use nimiq_block_production::BlockProducer;
use nimiq_blockchain::Blockchain;
use nimiq_database::{FromDatabaseValue, IntoDatabaseValue};
use nimiq_primitives::slots::Validators;
use nimiq_tendermint::{TendermintOutsideDeps, TendermintReturn, TendermintState};
use nimiq_validator_network::ValidatorNetwork;
use nimiq_vrf::VrfSeed;

use crate::tendermint::TendermintInterface;

pub(crate) struct PersistedMacroState<TValidatorNetwork: ValidatorNetwork + 'static>(
    pub  TendermintState<
        <TendermintInterface<TValidatorNetwork> as TendermintOutsideDeps>::ProposalTy,
        <TendermintInterface<TValidatorNetwork> as TendermintOutsideDeps>::ProposalCacheTy,
        <TendermintInterface<TValidatorNetwork> as TendermintOutsideDeps>::ProposalHashTy,
        <TendermintInterface<TValidatorNetwork> as TendermintOutsideDeps>::ProofTy,
    >,
);

// Don't know why this is necessary. #[derive(Clone)] does not work.
impl<TValidatorNetwork: ValidatorNetwork + 'static> Clone
    for PersistedMacroState<TValidatorNetwork>
{
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<TValidatorNetwork: ValidatorNetwork> IntoDatabaseValue
    for PersistedMacroState<TValidatorNetwork>
{
    fn database_byte_size(&self) -> usize {
        self.0.serialized_size()
    }

    fn copy_into_database(&self, mut bytes: &mut [u8]) {
        Serialize::serialize(&self.0, &mut bytes).unwrap();
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
        Ok(Self(Deserialize::deserialize(&mut cursor)?))
    }
}

pub(crate) struct ProduceMacroBlock<TValidatorNetwork: ValidatorNetwork + 'static> {
    tendermint: BoxStream<'static, TendermintReturn<TendermintInterface<TValidatorNetwork>>>,
}

impl<TValidatorNetwork: ValidatorNetwork + 'static> ProduceMacroBlock<TValidatorNetwork> {
    pub fn new(
        blockchain: Arc<RwLock<Blockchain>>,
        network: Arc<TValidatorNetwork>,
        block_producer: BlockProducer,
        validator_slot_band: u16,
        active_validators: Validators,
        prev_seed: VrfSeed,
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
        let deps = TendermintInterface::new(
            validator_slot_band,
            active_validators,
            prev_seed,
            block_height,
            network,
            blockchain,
            block_producer,
            proposal_stream,
            initial_round,
        );

        let state_opt = state.map(|s| s.0).filter(|s| s.height == block_height);

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

impl<TValidatorNetwork: ValidatorNetwork + 'static> Stream
    for ProduceMacroBlock<TValidatorNetwork>
{
    type Item = TendermintReturn<TendermintInterface<TValidatorNetwork>>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.tendermint.poll_next_unpin(cx)
    }
}
