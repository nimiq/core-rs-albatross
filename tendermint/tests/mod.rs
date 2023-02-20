pub mod common;

use nimiq_tendermint::{ProposalMessage, Protocol, SignedProposalMessage};

use self::common::{helper::run_tendermint, TestInherent, TestProposal, Validator};

#[tokio::test]
/// This instance proposes/receives a proposal for round 0 and also sees 2f+1 prevotes and precommits
/// and thus produces decision.
async fn it_produces_a_decision() {
    let decision = run_tendermint(
        vec![
            (true, None), // propose in round 0
        ],
        vec![vec![(Some(0), 0..Validator::TWO_F_PLUS_ONE)]],
        vec![vec![(Some(0), 0..Validator::TWO_F_PLUS_ONE)]],
        vec![],
    )
    .await
    .expect("");

    assert_eq!(decision.round, 0);
    assert_eq!(decision.proposal, TestProposal(0));
    assert_eq!(decision.inherents, TestInherent(0));
    assert!(decision.sig.len() >= Validator::TWO_F_PLUS_ONE);

    // Same setup just a proposal from somebody else.
    let decision = run_tendermint(
        vec![
            // Do not propose in round 0 but receive valid proposal
            (
                false,
                Some(SignedProposalMessage {
                    signature: true,
                    message: ProposalMessage {
                        proposal: TestProposal(3),
                        round: 0,
                        valid_round: None,
                    },
                }),
            ),
        ],
        vec![vec![(Some(3), 0..Validator::TWO_F_PLUS_ONE)]],
        vec![vec![(Some(3), 0..Validator::TWO_F_PLUS_ONE)]],
        vec![],
    )
    .await
    .expect("");

    assert_eq!(decision.round, 0);
    assert_eq!(decision.proposal, TestProposal(3));
    assert_eq!(decision.inherents, TestInherent(3));
    assert!(decision.sig.len() >= Validator::TWO_F_PLUS_ONE)
}

#[tokio::test]
/// This instance receives no proposal in round 0 and then 2f+1 prevotes and precommits against all proposals.
/// In round 1 it proposes/receives a proposal and sees 2f+1 prevotes and precommits for the proposal, thus producing the decision.
async fn it_deals_with_no_proposal() {
    let decision = run_tendermint(
        vec![
            (false, None), // propose in round 0
            (true, None),
        ],
        vec![
            vec![(None, 0..Validator::TWO_F_PLUS_ONE)],
            vec![(Some(1), 0..Validator::TWO_F_PLUS_ONE)],
        ],
        vec![
            vec![(None, 0..Validator::TWO_F_PLUS_ONE)],
            vec![(Some(1), 0..Validator::TWO_F_PLUS_ONE)],
        ],
        vec![],
    )
    .await
    .expect("");

    assert_eq!(decision.round, 1);
    assert_eq!(decision.proposal, TestProposal(1));
    assert_eq!(decision.inherents, TestInherent(1));
    assert!(decision.sig.len() >= Validator::TWO_F_PLUS_ONE);

    // Same setup, but 2nd proposal is from someone else.
    let decision = run_tendermint(
        vec![
            (false, None),
            // Do not propose in round 1 but receive valid proposal
            (
                false,
                Some(SignedProposalMessage {
                    signature: true,
                    message: ProposalMessage {
                        proposal: TestProposal(3),
                        round: 1,
                        valid_round: None,
                    },
                }),
            ),
        ],
        vec![
            vec![(None, 0..Validator::TWO_F_PLUS_ONE)],
            vec![(Some(3), 0..Validator::TWO_F_PLUS_ONE)],
        ],
        vec![
            vec![(None, 0..Validator::TWO_F_PLUS_ONE)],
            vec![(Some(3), 0..Validator::TWO_F_PLUS_ONE)],
        ],
        vec![],
    )
    .await
    .expect("");

    assert_eq!(decision.round, 1);
    assert_eq!(decision.proposal, TestProposal(3));
    assert_eq!(decision.inherents, TestInherent(3));
    assert!(decision.sig.len() >= Validator::TWO_F_PLUS_ONE)
}

#[tokio::test]
/// In round 0 the node locks itself on a proposal without producing the decision.
///
/// In round 1 the node re-proposes the proposal from round 0 and ultimately produce the decision.
async fn it_locks_and_rebroadcasts() {
    let decision = run_tendermint(
        vec![
            // Do not propose in round 0 but receive valid proposal
            (
                false,
                Some(SignedProposalMessage {
                    signature: true,
                    message: ProposalMessage {
                        proposal: TestProposal(3),
                        round: 0,
                        valid_round: None,
                    },
                }),
            ),
            // Do propose in round 1.
            (true, None),
        ],
        vec![
            // round 0 prevote is good
            vec![(Some(3), 0..Validator::TWO_F_PLUS_ONE)],
            // round 1 prevote is good
            vec![(Some(3), 0..Validator::TWO_F_PLUS_ONE)],
        ],
        vec![
            // round 0 precommit is conclusive None
            vec![
                (Some(3), 0..Validator::F_PLUS_ONE),
                (None, Validator::F_PLUS_ONE..(2 * Validator::F_PLUS_ONE)),
            ],
            // round 1 precommit is good
            vec![(Some(3), 0..Validator::TWO_F_PLUS_ONE)],
        ],
        vec![],
    )
    .await
    .expect("");

    assert_eq!(decision.round, 1);
    assert_eq!(decision.proposal, TestProposal(3));
    assert_eq!(decision.inherents, TestInherent(3));
    assert!(decision.sig.len() >= Validator::TWO_F_PLUS_ONE)
}

#[tokio::test]
/// Here the node locks itself in round 1 without producing a decision.
///
/// In round 2 it does not receive a proposal, and fails to request it in time.
/// Thus not voting for the proposal, even though it has enough precommits.
///
/// In round 2 it re-proposes the proposal it is locked on which ultimately produces a decision.
async fn it_does_not_unlock() {
    let decision = run_tendermint(
        vec![
            // Do not propose in round 0 but receive valid proposal
            (
                false,
                Some(SignedProposalMessage {
                    signature: true,
                    message: ProposalMessage {
                        proposal: TestProposal(3),
                        round: 0,
                        valid_round: None,
                    },
                }),
            ),
            // Do not propose in round 1 and also do not receive any proposal.
            (false, None),
            // Do propose in round 2
            (true, None),
        ],
        vec![
            // round 0 prevote is good, this should lock
            vec![(Some(3), 0..Validator::TWO_F_PLUS_ONE)],
            // round 1 prevote is good, however this node does not know `5` thus votes None, locked remains unchanged
            vec![(Some(5), 0..Validator::TWO_F_PLUS_ONE)],
            // As the instance should have re-proposed `3` this should lock again
            vec![(Some(3), 0..Validator::TWO_F_PLUS_ONE)],
        ],
        vec![
            // round 0 precommit is conclusive None
            vec![
                (Some(3), 0..Validator::F_PLUS_ONE),
                (None, Validator::F_PLUS_ONE..(2 * Validator::F_PLUS_ONE)),
            ],
            // round 1 precommit is conclusive None
            vec![
                (Some(5), 0..Validator::F_PLUS_ONE),
                (None, Validator::F_PLUS_ONE..(2 * Validator::F_PLUS_ONE)),
            ],
            // this should produce the decision
            vec![(Some(3), 0..Validator::TWO_F_PLUS_ONE)],
        ],
        vec![],
    )
    .await
    .expect("");

    assert_eq!(decision.round, 2);
    assert_eq!(decision.proposal, TestProposal(3));
    assert_eq!(decision.inherents, TestInherent(3));
    assert!(decision.sig.len() >= Validator::TWO_F_PLUS_ONE)
}

#[tokio::test]
/// Everything here is a timeout for round 0. It must not stall, but produce a decision in round 1 instead.
async fn it_can_deal_with_timeouts() {
    let decision = run_tendermint(
        vec![
            // Do not propose in round 0, also do not receive any proposal.
            (false, None),
            // Do propose in round 1
            (true, None),
        ],
        vec![
            // round 0 prevote is an improvable aggregate, which never improves.
            vec![
                (Some(2), 0..Validator::F_PLUS_ONE - 1), // F
                (
                    Some(3),
                    Validator::F_PLUS_ONE - 1..2 * (Validator::F_PLUS_ONE - 1),
                ), // + F
                (
                    None,
                    2 * (Validator::F_PLUS_ONE - 1)..(2 * (Validator::F_PLUS_ONE - 1) + 1),
                ), // + 1 = 2f+1
            ],
            // round 1 prevote is good
            vec![(Some(1), 0..Validator::TWO_F_PLUS_ONE)],
        ],
        vec![
            // round 0 precommit is an improvable aggregate, which never improves.
            vec![
                (Some(2), 0..Validator::F_PLUS_ONE - 1), // F
                (
                    Some(3),
                    Validator::F_PLUS_ONE - 1..2 * (Validator::F_PLUS_ONE - 1),
                ), // + F
                (
                    None,
                    2 * (Validator::F_PLUS_ONE - 1)..(2 * (Validator::F_PLUS_ONE - 1) + 1),
                ), // + 1 = 2f+1
            ],
            // round 1 precommit is good
            vec![(Some(1), 0..Validator::TWO_F_PLUS_ONE)],
        ],
        vec![],
    )
    .await
    .expect("");

    assert_eq!(decision.round, 1);
    assert_eq!(decision.proposal, TestProposal(1));
    assert_eq!(decision.inherents, TestInherent(1));
    assert!(decision.sig.len() >= Validator::TWO_F_PLUS_ONE)
}

#[tokio::test]
/// After insufficient prevotes in round 0, this node votes none until it can produce a decision in round 1
async fn it_can_deal_with_insufficient_prevotes() {
    let decision = run_tendermint(
        vec![
            // Do not propose in round 0 but receive valid proposal
            (
                false,
                Some(SignedProposalMessage {
                    signature: true,
                    message: ProposalMessage {
                        proposal: TestProposal(3),
                        round: 0,
                        valid_round: None,
                    },
                }),
            ),
            // Do propose in round 1
            (true, None),
        ],
        vec![
            // round 0 prevote is a conclusive aggregate, with mixed votes resulting in None
            vec![
                (Some(3), 0..Validator::TWO_F_PLUS_ONE - 1),
                (
                    None,
                    Validator::TWO_F_PLUS_ONE - 1
                        ..Validator::TWO_F_PLUS_ONE + Validator::F_PLUS_ONE - 1,
                ),
            ],
            // round 1 prevote is good
            vec![(Some(1), 0..Validator::TWO_F_PLUS_ONE)],
        ],
        vec![
            // round 0 precommit is a conclusive aggregate, with mixed votes resulting in None
            vec![
                (Some(3), 0..Validator::TWO_F_PLUS_ONE - 1),
                (
                    None,
                    Validator::TWO_F_PLUS_ONE - 1
                        ..Validator::TWO_F_PLUS_ONE + Validator::F_PLUS_ONE - 1,
                ),
            ],
            // round 1 prevote is good
            vec![(Some(1), 0..Validator::TWO_F_PLUS_ONE)],
        ],
        vec![],
    )
    .await
    .expect("");

    assert_eq!(decision.round, 1);
    assert_eq!(decision.proposal, TestProposal(1));
    assert_eq!(decision.inherents, TestInherent(1));
    assert!(decision.sig.len() >= Validator::TWO_F_PLUS_ONE)
}

#[tokio::test]
/// After locking itself in round 0, this node sees an insufficient precommit aggregation.
/// But since the next round produces a proposal with 2f+1 precommit votes it still produces the decision.
async fn it_can_deal_with_insufficient_precommits() {
    let decision = run_tendermint(
        vec![
            // Do not propose in round 0 but receive valid proposal
            (
                false,
                Some(SignedProposalMessage {
                    signature: true,
                    message: ProposalMessage {
                        proposal: TestProposal(3),
                        round: 0,
                        valid_round: None,
                    },
                }),
            ),
            // Do not propose in round 1 but receive valid proposal
            (
                false,
                Some(SignedProposalMessage {
                    signature: true,
                    message: ProposalMessage {
                        proposal: TestProposal(7),
                        round: 1,
                        valid_round: None,
                    },
                }),
            ),
        ],
        vec![
            // round 0 prevote is a conclusive aggregate, with mixed votes resulting in None
            vec![(Some(3), 0..Validator::TWO_F_PLUS_ONE)],
            // round 1 prevote is good
            vec![(Some(7), 0..Validator::TWO_F_PLUS_ONE)],
        ],
        vec![
            // round 0 precommit is a conclusive aggregate, with mixed votes resulting in None
            vec![
                (Some(3), 0..Validator::TWO_F_PLUS_ONE - 1),
                (
                    None,
                    Validator::TWO_F_PLUS_ONE - 1
                        ..Validator::TWO_F_PLUS_ONE + Validator::F_PLUS_ONE - 1,
                ),
            ],
            // round 1 prevote is good
            vec![(Some(7), 0..Validator::TWO_F_PLUS_ONE)],
        ],
        vec![],
    )
    .await
    .expect("");

    assert_eq!(decision.round, 1);
    assert_eq!(decision.proposal, TestProposal(7));
    assert_eq!(decision.inherents, TestInherent(7));
    assert!(decision.sig.len() >= Validator::TWO_F_PLUS_ONE)
}
