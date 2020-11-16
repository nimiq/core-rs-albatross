use async_trait::async_trait;
use beserial::Serialize;
use futures::{pin_mut, StreamExt};
use nimiq_hash::{Blake2sHash, Hash, SerializeContent};
use nimiq_primitives::policy::SLOTS;
use nimiq_tendermint::*;
use std::collections::BTreeMap;
use std::io;

// We need to implement Hash for the proposal type so we need our own type.
// round is only for testing, tendermint should ignore it completely.
#[derive(Copy, Clone, Debug, Eq)]
// proposal id (A or B) + validity + round
pub struct TestProposal(char, bool, u32);

// We only check equality for the proposal id (not the validity or round).
impl PartialEq for TestProposal {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

// We only serialize the proposal id (not the validity or round). char doesn't implement serialize,
// so we convert to a u32.
impl SerializeContent for TestProposal {
    fn serialize_content<W: io::Write>(&self, writer: &mut W) -> io::Result<usize> {
        let size = self.0.to_digit(36).unwrap().serialize(writer)?;
        Ok(size)
    }
}

impl Hash for TestProposal {}

// We can use this to mimic a validator. It determines which messages a single validator sees.
pub struct TestValidators {
    valid_init_state: bool,
    proposer_round: u32,
    // tuple is timeout + proposal + valid round proposer
    proposals_round: Vec<(bool, TestProposal, Option<u32>)>,
    // tuple is new round + votes for A/B/nil
    agg_prevote_round: Vec<(bool, u16, u16, u16)>,
    // tuple is new round + votes for A/B/nil
    agg_precommit_round: Vec<(bool, u16, u16, u16)>,
    // tuple is new round + votes for A/B/nil
    get_agg_round: Vec<(bool, u16, u16, u16)>,
}

#[async_trait]
impl TendermintOutsideDeps for TestValidators {
    type ProposalTy = TestProposal;
    // We never verify the proofs inside Tendermint.
    type ProofTy = ();
    type ResultTy = (TestProposal, ());

    fn verify_state(&self, _state: &TendermintState<Self::ProposalTy, Self::ProofTy>) -> bool {
        self.valid_init_state
    }

    fn is_our_turn(&self, round: u32) -> bool {
        self.proposer_round == round
    }

    fn is_valid(&self, proposal: Self::ProposalTy) -> bool {
        proposal.1
    }

    fn get_value(&self, round: u32) -> Result<Self::ProposalTy, TendermintError> {
        Ok(self.proposals_round[round as usize].1)
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
                self.proposals_round[round as usize].1,
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
        let a_hash = TestProposal('A', true, 0).hash();
        let b_hash = TestProposal('B', true, 0).hash();

        if step == Step::Prevote {
            return if self.agg_prevote_round[round as usize].0 {
                Ok(AggregationResult::NewRound(round + 1))
            } else {
                let mut agg = BTreeMap::new();
                agg.insert(
                    Some(a_hash),
                    ((), self.agg_prevote_round[round as usize].1 as usize),
                );
                agg.insert(
                    Some(b_hash),
                    ((), self.agg_prevote_round[round as usize].2 as usize),
                );
                agg.insert(
                    None,
                    ((), self.agg_prevote_round[round as usize].3 as usize),
                );
                Ok(AggregationResult::Aggregation(agg))
            };
        }

        if step == Step::Precommit {
            return if self.agg_precommit_round[round as usize].0 {
                Ok(AggregationResult::NewRound(round + 1))
            } else {
                let mut agg = BTreeMap::new();
                agg.insert(
                    Some(a_hash),
                    ((), self.agg_precommit_round[round as usize].1 as usize),
                );
                agg.insert(
                    Some(b_hash),
                    ((), self.agg_precommit_round[round as usize].2 as usize),
                );
                agg.insert(
                    None,
                    ((), self.agg_precommit_round[round as usize].3 as usize),
                );
                Ok(AggregationResult::Aggregation(agg))
            };
        }

        Err(TendermintError::AggregationError)
    }

    fn get_aggregation(
        &self,
        round: u32,
        _step: Step,
    ) -> Result<AggregationResult<Self::ProofTy>, TendermintError> {
        let a_hash = TestProposal('A', true, 0).hash();
        let b_hash = TestProposal('B', true, 0).hash();

        if self.get_agg_round[round as usize].0 {
            Ok(AggregationResult::NewRound(round + 1))
        } else {
            let mut agg = BTreeMap::new();
            agg.insert(
                Some(a_hash),
                ((), self.get_agg_round[round as usize].1 as usize),
            );
            agg.insert(
                Some(b_hash),
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
        valid_init_state: true,
        proposer_round: 0,
        proposals_round: vec![(false, TestProposal('A', true, 0), None)],
        agg_prevote_round: vec![(false, SLOTS, 0, 0)],
        agg_precommit_round: vec![(false, SLOTS, 0, 0)],
        get_agg_round: vec![],
    };

    let tendermint = expect_block(proposer, None);
    pin_mut!(tendermint);

    while let Some(value) = tendermint.next().await {
        match value {
            // We need to deconstruct TestProposal because we just implemented a PartialEq trait that
            // ignores the last two fields.
            TendermintReturn::Result(block) => assert!(block.0.0=='A' && block.0.1 && block.0.2==0),
            TendermintReturn::StateUpdate(state) => println!("{:?}\n", state),
            TendermintReturn::Error(_) => panic!(),
        }
    }

    // from the perspective of another validator
    let validator = TestValidators {
        valid_init_state: true,
        proposer_round: 99,
        proposals_round: vec![(false, TestProposal('A', true, 0), None)],
        agg_prevote_round: vec![(false, SLOTS, 0, 0)],
        agg_precommit_round: vec![(false, SLOTS, 0, 0)],
        get_agg_round: vec![],
    };

    let tendermint = expect_block(validator, None);
    pin_mut!(tendermint);

    while let Some(value) = tendermint.next().await {
        match value {
            TendermintReturn::Result(block) => assert!(block.0.0=='A' && block.0.1 && block.0.2==0),
            TendermintReturn::StateUpdate(state) => println!("{:?}\n", state),
            TendermintReturn::Error(_) => panic!(),
        }
    }
}

#[tokio::test]
async fn invalid_proposal() {
    // from the perspective of another validator
    let validator = TestValidators {
        valid_init_state: true,
        proposer_round: 99,
        proposals_round: vec![
            (false, TestProposal('A', false, 0), None),
            (false, TestProposal('A', true, 1), None),
        ],
        agg_prevote_round: vec![(false, SLOTS, 0, 0), (false, SLOTS, 0, 0)],
        agg_precommit_round: vec![(false, SLOTS, 0, 0), (false, SLOTS, 0, 0)],
        get_agg_round: vec![],
    };

    let tendermint = expect_block(validator, None);
    pin_mut!(tendermint);

    while let Some(value) = tendermint.next().await {
        match value {
            TendermintReturn::Result(block) => assert!(block.0.0=='A' && block.0.1 && block.0.2==1),
            TendermintReturn::StateUpdate(state) => println!("{:?}\n", state),
            TendermintReturn::Error(_) => panic!(),
        }
    }
}

#[tokio::test]
async fn no_proposal() {
    // from the perspective of another validator
    let validator = TestValidators {
        valid_init_state: true,
        proposer_round: 99,
        proposals_round: vec![
            (true, TestProposal('A', true, 0), None),
            (false, TestProposal('A', true, 1), None),
        ],
        agg_prevote_round: vec![(false, 0, 0, SLOTS), (false, SLOTS, 0, 0)],
        agg_precommit_round: vec![(false, 0, 0, SLOTS), (false, SLOTS, 0, 0)],
        get_agg_round: vec![],
    };

    let tendermint = expect_block(validator, None);
    pin_mut!(tendermint);

    while let Some(value) = tendermint.next().await {
        match value {
            TendermintReturn::Result(block) => assert!(block.0.0=='A' && block.0.1 && block.0.2==1),
            TendermintReturn::StateUpdate(state) => println!("{:?}\n", state),
            TendermintReturn::Error(_) => panic!(),
        }
    }
}

#[tokio::test]
async fn all_timeouts() {
    // from the perspective of another validator
    let validator = TestValidators {
        valid_init_state: true,
        proposer_round: 99,
        proposals_round: vec![
            (true, TestProposal('A', true, 0), None),
            (false, TestProposal('A', true, 1), None),
        ],
        agg_prevote_round: vec![(true, 0, 0, SLOTS), (false, SLOTS, 0, 0)],
        agg_precommit_round: vec![(true, 0, 0, SLOTS), (false, SLOTS, 0, 0)],
        get_agg_round: vec![],
    };

    let tendermint = expect_block(validator, None);
    pin_mut!(tendermint);

    while let Some(value) = tendermint.next().await {
        match value {
            TendermintReturn::Result(block) => assert!(block.0.0=='A' && block.0.1 && block.0.2==1),
            TendermintReturn::StateUpdate(state) => println!("{:?}\n", state),
            TendermintReturn::Error(_) => panic!(),
        }
    }
}

#[tokio::test]
async fn not_enough_prevotes() {
    // from the perspective of another validator
    let validator = TestValidators {
        valid_init_state: true,
        proposer_round: 99,
        proposals_round: vec![
            (false, TestProposal('A', true, 0), None),
            (false, TestProposal('A', true, 1), None),
        ],
        agg_prevote_round: vec![(false, SLOTS / 2, SLOTS / 2, 0), (false, SLOTS, 0, 0)],
        agg_precommit_round: vec![(false, 0, 0, SLOTS), (false, SLOTS, 0, 0)],
        get_agg_round: vec![],
    };

    let tendermint = expect_block(validator, None);
    pin_mut!(tendermint);

    while let Some(value) = tendermint.next().await {
        match value {
            TendermintReturn::Result(block) => assert!(block.0.0=='A' && block.0.1 && block.0.2==1),
            TendermintReturn::StateUpdate(state) => println!("{:?}\n", state),
            TendermintReturn::Error(_) => panic!(),
        }
    }
}

#[tokio::test]
async fn not_enough_precommits() {
    // from the perspective of another validator
    let validator = TestValidators {
        valid_init_state: true,
        proposer_round: 99,
        proposals_round: vec![
            (false, TestProposal('A', true, 0), None),
            (false, TestProposal('A', true, 1), None),
        ],
        agg_prevote_round: vec![(false, SLOTS, 0, 0), (false, SLOTS, 0, 0)],
        agg_precommit_round: vec![(false, SLOTS / 2, SLOTS / 2, 0), (false, SLOTS, 0, 0)],
        get_agg_round: vec![],
    };

    let tendermint = expect_block(validator, None);
    pin_mut!(tendermint);

    while let Some(value) = tendermint.next().await {
        match value {
            TendermintReturn::Result(block) => assert!(block.0.0=='A' && block.0.1 && block.0.2==1),
            TendermintReturn::StateUpdate(state) => println!("{:?}\n", state),
            TendermintReturn::Error(_) => panic!(),
        }
    }
}
