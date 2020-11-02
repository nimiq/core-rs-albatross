use async_trait::async_trait;
use beserial::{Deserialize, Serialize};
use futures::{StreamExt, TryStreamExt};
use nimiq_network_interface::message::Message;
use nimiq_network_interface::network::{
    Network as NetworkInterface, Network, NetworkEvent, ReceiveFromAll,
};
use nimiq_network_mock::network::MockNetwork;
use nimiq_tendermint::protocol::TendermintReturn;
use nimiq_tendermint::{
    PrecommitAggregationResult, PrevoteAggregationResult, SingleDecision, Tendermint,
    TendermintOutsideDeps,
};
use parking_lot::RwLock;
use std::sync::Arc;
use tokio::sync::broadcast::Receiver;
use tokio::time::{delay_for, Duration};

pub struct Deps1 {}

#[async_trait]
impl TendermintOutsideDeps for Deps1 {
    type ProposalTy = u32;
    type ProofTy = u32;
    type ResultTy = (u32, u32);

    fn is_our_turn(&self, round: u32) -> bool {
        true
    }

    fn verify_proposal(&self, proposal: Arc<Self::ProposalTy>, round: u32) -> bool {
        true
    }

    fn produce_proposal(&self, round: u32) -> Option<Self::ProposalTy> {
        Some(483)
    }

    fn assemble_block(
        &self,
        proposal: Arc<Self::ProposalTy>,
        proof: Self::ProofTy,
    ) -> Self::ResultTy {
        (*proposal, proof)
    }

    async fn broadcast_prevote(
        &self,
        proposal: Option<Arc<Self::ProposalTy>>,
        round: u32,
        decision: &SingleDecision,
    ) -> PrevoteAggregationResult<Self::ProofTy> {
        PrevoteAggregationResult::Polka(0)
    }

    async fn broadcast_precommit(
        &self,
        proposal: Option<Arc<Self::ProposalTy>>,
        round: u32,
        decision: &SingleDecision,
    ) -> PrecommitAggregationResult<Self::ProofTy> {
        PrecommitAggregationResult::Block2Fp1(6364)
    }

    fn verify_proposal_state(&self, round: u32, previous_precommit_result: &Self::ProofTy) -> bool {
        unimplemented!()
    }

    fn verify_prevote_state(
        &self,
        round: u32,
        proposal: Arc<Self::ProposalTy>,
        previous_precommit_result: &Self::ProofTy,
    ) -> bool {
        unimplemented!()
    }

    fn verify_precommit_state(
        &self,
        round: u32,
        proposal: Option<Arc<Self::ProposalTy>>,
        prevote_result: &Self::ProofTy,
    ) -> Option<SingleDecision> {
        unimplemented!()
    }
}

#[tokio::test]
async fn we_propose_and_everything_works() {
    let t = Tendermint::new(Deps1 {}, MockNetwork::new(1));

    while let Some(r) = t.expect_block().next().await {
        if let TendermintReturn::Result(block) = r {
            assert_eq!(block, (483, 6364));
            return;
        }
    }

    assert!(false);
}

pub struct Deps2 {}

#[async_trait]
impl TendermintOutsideDeps for Deps2 {
    type ProposalTy = u32;
    type ProofTy = u32;
    type ResultTy = (u32, u32);

    fn is_our_turn(&self, round: u32) -> bool {
        true
    }

    fn verify_proposal(&self, proposal: Arc<Self::ProposalTy>, round: u32) -> bool {
        true
    }

    fn produce_proposal(&self, round: u32) -> Option<Self::ProposalTy> {
        Some(100 + round)
    }

    fn assemble_block(
        &self,
        proposal: Arc<Self::ProposalTy>,
        proof: Self::ProofTy,
    ) -> Self::ResultTy {
        (*proposal, proof)
    }

    async fn broadcast_prevote(
        &self,
        proposal: Option<Arc<Self::ProposalTy>>,
        round: u32,
        decision: &SingleDecision,
    ) -> PrevoteAggregationResult<Self::ProofTy> {
        assert_eq!(*decision, SingleDecision::Block);
        if round < 10 {
            PrevoteAggregationResult::Other(0)
        } else {
            PrevoteAggregationResult::Polka(0)
        }
    }

    async fn broadcast_precommit(
        &self,
        proposal: Option<Arc<Self::ProposalTy>>,
        round: u32,
        decision: &SingleDecision,
    ) -> PrecommitAggregationResult<Self::ProofTy> {
        if round < 10 {
            // There was a nil polka so there must be a nil precommit
            assert_eq!(*decision, SingleDecision::Nil);
            PrecommitAggregationResult::Other(12345678)
        } else {
            PrecommitAggregationResult::Block2Fp1(210)
        }
    }

    fn verify_proposal_state(&self, round: u32, previous_precommit_result: &Self::ProofTy) -> bool {
        unimplemented!()
    }

    fn verify_prevote_state(
        &self,
        round: u32,
        proposal: Arc<Self::ProposalTy>,
        previous_precommit_result: &Self::ProofTy,
    ) -> bool {
        unimplemented!()
    }

    fn verify_precommit_state(
        &self,
        round: u32,
        proposal: Option<Arc<Self::ProposalTy>>,
        prevote_result: &Self::ProofTy,
    ) -> Option<SingleDecision> {
        unimplemented!()
    }
}

#[tokio::test]
async fn we_propose_and_first_rounds_nil() {
    let t = Tendermint::new(Deps2 {}, MockNetwork::new(1));

    let mut stream = t.expect_block();
    while let Some(r) = stream.next().await {
        if let TendermintReturn::Result(block) = r {
            assert_eq!(block, (110, 210));
            return;
        }
    }

    assert!(false);
}

pub struct Deps3 {
    validator: u32,
}

#[async_trait]
impl TendermintOutsideDeps for Deps3 {
    type ProposalTy = u32;
    type ProofTy = u32;
    type ResultTy = (u32, u32);

    fn is_our_turn(&self, round: u32) -> bool {
        return self.validator == 2;
    }

    fn verify_proposal(&self, proposal: Arc<Self::ProposalTy>, round: u32) -> bool {
        true
    }

    fn produce_proposal(&self, round: u32) -> Option<Self::ProposalTy> {
        Some(100 + 10 * round + self.validator)
    }

    fn assemble_block(
        &self,
        proposal: Arc<Self::ProposalTy>,
        proof: Self::ProofTy,
    ) -> Self::ResultTy {
        (*proposal, proof)
    }

    async fn broadcast_prevote(
        &self,
        proposal: Option<Arc<Self::ProposalTy>>,
        round: u32,
        decision: &SingleDecision,
    ) -> PrevoteAggregationResult<Self::ProofTy> {
        PrevoteAggregationResult::Polka(0)
    }

    async fn broadcast_precommit(
        &self,
        proposal: Option<Arc<Self::ProposalTy>>,
        round: u32,
        decision: &SingleDecision,
    ) -> PrecommitAggregationResult<Self::ProofTy> {
        PrecommitAggregationResult::Block2Fp1(210)
    }

    fn verify_proposal_state(&self, round: u32, previous_precommit_result: &Self::ProofTy) -> bool {
        unimplemented!()
    }

    fn verify_prevote_state(
        &self,
        round: u32,
        proposal: Arc<Self::ProposalTy>,
        previous_precommit_result: &Self::ProofTy,
    ) -> bool {
        unimplemented!()
    }

    fn verify_precommit_state(
        &self,
        round: u32,
        proposal: Option<Arc<Self::ProposalTy>>,
        prevote_result: &Self::ProofTy,
    ) -> Option<SingleDecision> {
        unimplemented!()
    }
}

#[tokio::test]
async fn two_validators() {
    let net1 = MockNetwork::new(1);
    let net2 = MockNetwork::new(2);
    net1.connect(&net2);

    let a = tokio::spawn(async move {
        let t = Tendermint::new(Deps3 { validator: 1 }, net1);

        while let Some(r) = t.expect_block().next().await {
            if let TendermintReturn::Result(block) = r {
                assert_eq!(block, (112, 210));
                return;
            }
        }

        assert!(false);
    });

    let b = tokio::spawn(async move {
        // delay_for(Duration::from_secs(2)).await;
        let t = Tendermint::new(Deps3 { validator: 2 }, net2);

        while let Some(r) = t.expect_block().next().await {
            if let TendermintReturn::Result(block) = r {
                assert_eq!(block, (112, 210));
                return;
            }
        }

        assert!(false);
    });

    a.await;
    b.await;
}
