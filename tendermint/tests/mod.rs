use async_trait::async_trait;
use beserial::Serialize;
use futures::{pin_mut, StreamExt};
use nimiq_hash::{Blake2sHash, Hash, SerializeContent};
use nimiq_primitives::policy::SLOTS;
use nimiq_tendermint::{
    expect_block, AggregationResult, ProposalResult, Step, TendermintError, TendermintOutsideDeps,
    TendermintReturn, TendermintState,
};
use std::collections::BTreeMap;
use std::io;

// We need to implement Hash for the proposal type so we need our own type.
// TODO: Change to bool. True is a valid proposal, false is an invalid one.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct TestProposal(u32);

impl SerializeContent for TestProposal {
    fn serialize_content<W: io::Write>(&self, writer: &mut W) -> io::Result<usize> {
        let size = self.0.serialize(writer)?;
        Ok(size)
    }
}

impl Hash for TestProposal {}

// We can use this to mimic a validator. The index is the validator index.
pub struct TestValidators1 {
    index: u16,
}

#[async_trait]
impl TendermintOutsideDeps for TestValidators1 {
    type ProposalTy = TestProposal;
    // We never verify the proofs inside Tendermint.
    type ProofTy = ();
    type ResultTy = (TestProposal, ());

    fn verify_state(&self, _state: &TendermintState<Self::ProposalTy, Self::ProofTy>) -> bool {
        unimplemented!()
    }

    fn is_our_turn(&self, round: u32) -> bool {
        self.index as u32 == round
    }

    fn is_valid(&self, _proposal: Self::ProposalTy) -> bool {
        true
    }

    fn get_value(&self, _round: u32) -> Result<Self::ProposalTy, TendermintError> {
        Ok(TestProposal(42))
    }

    fn assemble_block(
        &self,
        proposal: Self::ProposalTy,
        proof: Self::ProofTy,
    ) -> Result<Self::ResultTy, TendermintError> {
        Ok((proposal, proof))
    }

    async fn broadcast_proposal(
        &self,
        _round: u32,
        _proposal: Self::ProposalTy,
        _valid_round: Option<u32>,
    ) -> Result<(), TendermintError> {
        Ok(())
    }

    async fn await_proposal(
        &self,
        _round: u32,
    ) -> Result<ProposalResult<Self::ProposalTy>, TendermintError> {
        Ok(ProposalResult::Proposal(TestProposal(42), None))
    }

    async fn broadcast_and_aggregate(
        &mut self,
        _round: u32,
        _step: Step,
        _proposal: Option<Blake2sHash>,
    ) -> Result<AggregationResult<Self::ProofTy>, TendermintError> {
        let proposal_hash = TestProposal(42).hash();

        let mut agg = BTreeMap::new();

        agg.insert(Some(proposal_hash), ((), SLOTS as usize));

        Ok(AggregationResult::Aggregation(agg))
    }

    fn get_aggregation(
        &self,
        _round: u32,
        _step: Step,
    ) -> Result<AggregationResult<Self::ProofTy>, TendermintError> {
        unimplemented!()
    }

    fn cancel_aggregation(&mut self, _round: u32, _step: Step) -> Result<(), TendermintError> {
        unimplemented!()
    }
}

#[tokio::test]
async fn everything_works() {
    let proposer = TestValidators1 { index: 0 };
    let tendermint = expect_block(proposer, None);

    pin_mut!(tendermint);

    while let Some(value) = tendermint.next().await {
        if let TendermintReturn::Error(_) = value {
            panic!()
        }
    }

    let validator = TestValidators1 { index: 1 };
    let tendermint = expect_block(validator, None);

    pin_mut!(tendermint);

    while let Some(value) = tendermint.next().await {
        if let TendermintReturn::Error(_) = value {
            panic!()
        }
    }
}

pub struct TestValidators2 {
    index: u16,
}

#[async_trait]
impl TendermintOutsideDeps for TestValidators2 {
    type ProposalTy = TestProposal;
    // We never verify the proofs inside Tendermint.
    type ProofTy = ();
    type ResultTy = (TestProposal, ());

    fn verify_state(&self, _state: &TendermintState<Self::ProposalTy, Self::ProofTy>) -> bool {
        unimplemented!()
    }

    fn is_our_turn(&self, round: u32) -> bool {
        self.index as u32 == round
    }

    fn is_valid(&self, _proposal: Self::ProposalTy) -> bool {
        true
    }

    fn get_value(&self, _round: u32) -> Result<Self::ProposalTy, TendermintError> {
        Ok(TestProposal(42))
    }

    fn assemble_block(
        &self,
        proposal: Self::ProposalTy,
        proof: Self::ProofTy,
    ) -> Result<Self::ResultTy, TendermintError> {
        Ok((proposal, proof))
    }

    async fn broadcast_proposal(
        &self,
        _round: u32,
        _proposal: Self::ProposalTy,
        _valid_round: Option<u32>,
    ) -> Result<(), TendermintError> {
        Ok(())
    }

    async fn await_proposal(
        &self,
        _round: u32,
    ) -> Result<ProposalResult<Self::ProposalTy>, TendermintError> {
        Ok(ProposalResult::Proposal(TestProposal(42), None))
    }

    async fn broadcast_and_aggregate(
        &mut self,
        _round: u32,
        _step: Step,
        _proposal: Option<Blake2sHash>,
    ) -> Result<AggregationResult<Self::ProofTy>, TendermintError> {
        let proposal_hash = TestProposal(42).hash();

        let mut agg = BTreeMap::new();

        agg.insert(Some(proposal_hash), ((), SLOTS as usize));

        Ok(AggregationResult::Aggregation(agg))
    }

    fn get_aggregation(
        &self,
        _round: u32,
        _step: Step,
    ) -> Result<AggregationResult<Self::ProofTy>, TendermintError> {
        unimplemented!()
    }

    fn cancel_aggregation(&mut self, _round: u32, _step: Step) -> Result<(), TendermintError> {
        unimplemented!()
    }
}

#[tokio::test]
async fn no_proposal() {
    let proposer = TestValidators1 { index: 0 };
    let tendermint = expect_block(proposer, None);

    pin_mut!(tendermint);

    while let Some(value) = tendermint.next().await {
        if let TendermintReturn::Error(_) = value {
            panic!()
        }
    }

    let validator = TestValidators1 { index: 1 };
    let tendermint = expect_block(validator, None);

    pin_mut!(tendermint);

    while let Some(value) = tendermint.next().await {
        if let TendermintReturn::Error(_) = value {
            panic!()
        }
    }
}
