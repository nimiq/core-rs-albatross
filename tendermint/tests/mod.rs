use async_trait::async_trait;
use beserial::Serialize;
use futures::{pin_mut, StreamExt};
use nimiq_hash::{Blake2sHash, Hash, SerializeContent};
use nimiq_primitives::policy::SLOTS;
use nimiq_tendermint::*;
use std::collections::BTreeMap;
use std::io;

// We need to implement Hash for the proposal type so we need our own type.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct TestProposal(bool);

impl SerializeContent for TestProposal {
    fn serialize_content<W: io::Write>(&self, writer: &mut W) -> io::Result<usize> {
        let size = self.0.serialize(writer)?;
        Ok(size)
    }
}

impl Hash for TestProposal {}

// We can use this to mimic a validator. It determines which messages a single validator sees.
pub struct TestValidators {
    valid_state: bool,
    proposer_round: u32,
    // tuple is timeout + proposal + valid round proposer
    proposals_round: Vec<(bool, bool, Option<u32>)>,
    // tuple is new round + votes for true/false/nil
    agg_prevote_round: Vec<(bool, u16, u16, u16)>,
    // tuple is new round + votes for true/false/nil
    agg_precommit_round: Vec<(bool, u16, u16, u16)>,
    // tuple is new round + votes for true/false/nil
    get_agg_round: Vec<(bool, u16, u16, u16)>,
}

#[async_trait]
impl TendermintOutsideDeps for TestValidators {
    type ProposalTy = TestProposal;
    // We never verify the proofs inside Tendermint.
    type ProofTy = ();
    type ResultTy = (TestProposal, ());

    fn verify_state(&self, _state: &TendermintState<Self::ProposalTy, Self::ProofTy>) -> bool {
        self.valid_state
    }

    fn is_our_turn(&self, round: u32) -> bool {
        self.proposer_round == round
    }

    fn is_valid(&self, proposal: Self::ProposalTy) -> bool {
        proposal.0
    }

    fn get_value(&self, round: u32) -> Result<Self::ProposalTy, TendermintError> {
        Ok(TestProposal(self.proposals_round[round as usize].1))
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
        round: u32,
    ) -> Result<ProposalResult<Self::ProposalTy>, TendermintError> {
        if self.proposals_round[round as usize].0 {
            Ok(ProposalResult::Timeout)
        } else {
            Ok(ProposalResult::Proposal(
                TestProposal(self.proposals_round[round as usize].1),
                self.proposals_round[round as usize].2,
            ))
        }
    }

    async fn broadcast_and_aggregate(
        &mut self,
        round: u32,
        step: Step,
        _proposal: Option<Blake2sHash>,
    ) -> Result<AggregationResult<Self::ProofTy>, TendermintError> {
        let true_hash = TestProposal(true).hash();
        let false_hash = TestProposal(false).hash();

        if step == Step::Prevote {
            if self.agg_prevote_round[round as usize].0 {
                return Ok(AggregationResult::NewRound(round + 1));
            } else {
                let mut agg = BTreeMap::new();
                agg.insert(
                    Some(true_hash),
                    ((), self.agg_prevote_round[round as usize].1 as usize),
                );
                agg.insert(
                    Some(false_hash),
                    ((), self.agg_prevote_round[round as usize].2 as usize),
                );
                agg.insert(
                    None,
                    ((), self.agg_prevote_round[round as usize].3 as usize),
                );
                return Ok(AggregationResult::Aggregation(agg));
            }
        }

        if step == Step::Precommit {
            if self.agg_precommit_round[round as usize].0 {
                return Ok(AggregationResult::NewRound(round + 1));
            } else {
                let mut agg = BTreeMap::new();
                agg.insert(
                    Some(true_hash),
                    ((), self.agg_precommit_round[round as usize].1 as usize),
                );
                agg.insert(
                    Some(false_hash),
                    ((), self.agg_precommit_round[round as usize].2 as usize),
                );
                agg.insert(
                    None,
                    ((), self.agg_precommit_round[round as usize].3 as usize),
                );
                return Ok(AggregationResult::Aggregation(agg));
            }
        }

        Err(TendermintError::AggregationError)
    }

    fn get_aggregation(
        &self,
        round: u32,
        _step: Step,
    ) -> Result<AggregationResult<Self::ProofTy>, TendermintError> {
        let true_hash = TestProposal(true).hash();
        let false_hash = TestProposal(false).hash();

        if self.get_agg_round[round as usize].0 {
            Ok(AggregationResult::NewRound(round + 1))
        } else {
            let mut agg = BTreeMap::new();
            agg.insert(
                Some(true_hash),
                ((), self.get_agg_round[round as usize].1 as usize),
            );
            agg.insert(
                Some(false_hash),
                ((), self.get_agg_round[round as usize].2 as usize),
            );
            agg.insert(None, ((), self.get_agg_round[round as usize].3 as usize));
            Ok(AggregationResult::Aggregation(agg))
        }
    }

    fn cancel_aggregation(&mut self, _round: u32, _step: Step) -> Result<(), TendermintError> {
        unimplemented!()
    }
}

#[tokio::test]
async fn everything_works() {
    // from the perspective of the proposer
    let proposer = TestValidators {
        valid_state: true,
        proposer_round: 0,
        proposals_round: vec![(false, true, None)],
        agg_prevote_round: vec![(false, SLOTS, 0, 0)],
        agg_precommit_round: vec![(false, SLOTS, 0, 0)],
        get_agg_round: vec![],
    };

    let tendermint = expect_block(proposer, None);
    pin_mut!(tendermint);

    while let Some(value) = tendermint.next().await {
        match value {
            TendermintReturn::Result(block) => assert!(block.0.0),
            TendermintReturn::StateUpdate(state) => println!("{:?}\n", state),
            TendermintReturn::Error(_) => panic!(),
        }
    }

    // from the perspective of another validator
    let validator = TestValidators {
        valid_state: true,
        proposer_round: 99,
        proposals_round: vec![(false, true, None)],
        agg_prevote_round: vec![(false, SLOTS, 0, 0)],
        agg_precommit_round: vec![(false, SLOTS, 0, 0)],
        get_agg_round: vec![],
    };

    let tendermint = expect_block(validator, None);
    pin_mut!(tendermint);

    while let Some(value) = tendermint.next().await {
        match value {
            TendermintReturn::Result(block) => assert!(block.0.0),
            TendermintReturn::StateUpdate(state) => println!("{:?}\n", state),
            TendermintReturn::Error(_) => panic!(),
        }
    }
}

// #[tokio::test]
// async fn no_proposal() {
//     let proposer = TestValidators { index: 0 };
//     let tendermint = expect_block(proposer, None);
//
//     pin_mut!(tendermint);
//
//     while let Some(value) = tendermint.next().await {
//         if let TendermintReturn::Error(_) = value {
//             panic!()
//         }
//     }
//
//     let validator = TestValidators { index: 1 };
//     let tendermint = expect_block(validator, None);
//
//     pin_mut!(tendermint);
//
//     while let Some(value) = tendermint.next().await {
//         if let TendermintReturn::Error(_) = value {
//             panic!()
//         }
//     }
// }
