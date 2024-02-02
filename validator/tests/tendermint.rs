use std::sync::Arc;

use nimiq_blockchain_interface::AbstractBlockchain;
use nimiq_network_libp2p::Network;
use nimiq_network_mock::MockHub;
use nimiq_primitives::{networks::NetworkId, policy::Policy};
use nimiq_tendermint::{ProposalMessage, Protocol, SignedProposalMessage};
use nimiq_test_log::test;
use nimiq_test_utils::{block_production::TemporaryBlockProducer, test_network::TestNetwork};
use nimiq_validator::{aggregation::tendermint::proposal::Header, tendermint::TendermintProtocol};
use nimiq_validator_network::network_impl::ValidatorNetworkImpl;

#[test(tokio::test)]
async fn it_verifies_inferior_chain_proposals() {
    let temp_producer1 = TemporaryBlockProducer::default();
    let temp_producer2 = TemporaryBlockProducer::default();
    let blockchain1 = Arc::clone(&temp_producer1.blockchain);
    let blockchain2 = Arc::clone(&temp_producer2.blockchain);

    // Progress until there are 2 micro blocks left before the macro block
    for _ in 0..Policy::blocks_per_batch() - 3 {
        let block = temp_producer1.next_block(vec![], false);
        temp_producer2
            .push(block)
            .expect("Should be able to push block");
    }

    assert_eq!(
        Policy::macro_block_after(blockchain1.read().head().block_number()),
        blockchain1.read().head().block_number() + 3,
    );
    assert_eq!(
        Policy::macro_block_after(blockchain2.read().head().block_number()),
        blockchain2.read().head().block_number() + 3,
    );

    // create but do not push a skip block to move all subsequent micro blocks to inferior chain.
    let first_skip_block = temp_producer1.next_block_no_push(vec![], true);
    // produce the next 2 micro blocks and push them such that the next block is a macro block.
    let second_to_last1 = temp_producer1.next_block(vec![], false);
    let second_to_last2 = temp_producer1.next_block(vec![], false);
    // create a proposal for the macro block, but do not produce a block.
    let inf_proposal1 = {
        let b = blockchain1.read();
        temp_producer1.producer.next_macro_block_proposal(
            &b,
            b.head().timestamp() + Policy::BLOCK_SEPARATION_TIME,
            0,
            vec![],
        )
    };
    // Use the skip block to rebranch the micro blocks away, effectively leaving the proposal on an inferior chain.
    // This chain has 2 blocks differ from the main chain preceding the macro block.
    temp_producer1
        .push(first_skip_block.clone())
        .expect("Pushing skip block should succeed.");
    temp_producer2
        .push(first_skip_block)
        .expect("Pushing skip block should succeed.");

    assert_eq!(
        Policy::macro_block_after(blockchain1.read().head().block_number()),
        blockchain1.read().head().block_number() + 2,
    );
    assert_eq!(
        Policy::macro_block_after(blockchain2.read().head().block_number()),
        blockchain2.read().head().block_number() + 2,
    );

    // Create but don't push another skip block to move the subsequent micro block to an inferior chain
    let second_skip_block = temp_producer1.next_block_no_push(vec![], true);
    // create the micro block and push it such that the next block is a macro block.
    let last = temp_producer1.next_block(vec![], false);
    // create a macro block proposal, but do not produce a block.
    let inf_proposal2 = {
        let b = blockchain1.read();
        temp_producer1.producer.next_macro_block_proposal(
            &b,
            b.head().timestamp() + Policy::BLOCK_SEPARATION_TIME,
            0,
            vec![],
        )
    };

    // use the skip block to move the proposal to inferior chain.
    temp_producer1
        .push(second_skip_block.clone())
        .expect("Pushing skip block should succeed.");
    temp_producer2
        .push(second_skip_block)
        .expect("Pushing skip block should succeed.");

    assert_eq!(
        Policy::macro_block_after(blockchain1.read().head().block_number()),
        blockchain1.read().head().block_number() + 1,
    );
    assert_eq!(
        Policy::macro_block_after(blockchain2.read().head().block_number()),
        blockchain2.read().head().block_number() + 1,
    );

    // Finally create a proposal on main chain
    let main_chain_proposal = {
        let b = blockchain1.read();
        temp_producer1.producer.next_macro_block_proposal(
            &b,
            b.head().timestamp() + Policy::BLOCK_SEPARATION_TIME,
            0,
            vec![],
        )
    };

    // blockchain2 here has not seen any of the inferior micro blocks nor any of the proposals.

    // Create TendermintProtocol for blockchain2
    let current_validators = blockchain1.read().current_validators().unwrap();
    let hub = MockHub::default();
    let nw: Arc<Network> = TestNetwork::build_network(0, Default::default(), &mut Some(hub)).await;
    let val_net = Arc::new(ValidatorNetworkImpl::new(nw));
    let interface = TendermintProtocol::new(
        Arc::clone(&blockchain2),
        val_net,
        temp_producer2.producer.clone(),
        current_validators,
        0,
        NetworkId::UnitAlbatross,
        blockchain2.read().head().block_number() + 1,
    );

    // Make sure the main chain proposal is acceptable.
    let main_chain_msg = ProposalMessage {
        round: 0,
        valid_round: None,
        proposal: Header(main_chain_proposal.header, None),
    };
    let main_chain_sig = interface.sign_proposal(&main_chain_msg);
    let message = SignedProposalMessage {
        message: main_chain_msg,
        signature: main_chain_sig,
    };

    assert!(interface.verify_proposal(&message, None).is_ok());

    // Make sure the second inferior chain proposal is not acceptable without the micro block
    let inf_chain2 = ProposalMessage {
        round: 0,
        valid_round: None,
        proposal: Header(inf_proposal2.header, None),
    };
    let inf_chain2_sig = interface.sign_proposal(&inf_chain2);
    let message: SignedProposalMessage<Header<_>, _> = SignedProposalMessage {
        message: inf_chain2,
        signature: inf_chain2_sig,
    };
    assert!(interface.verify_proposal(&message, None).is_err());

    // push the micro block and then make sure the proposal is ok
    temp_producer2
        .push(last)
        .expect("Pushing inferior micro block should succeed.");
    let body = interface
        .verify_proposal(&message, None)
        .expect("Verification must succeed.");

    assert_eq!(inf_proposal2.body.expect(""), body.0);

    // Make sure the first inferior chain proposal is not acceptable without both the micro blocks
    let inf_chain1 = ProposalMessage {
        round: 0,
        valid_round: None,
        proposal: Header(inf_proposal1.header.clone(), None),
    };
    let inf_chain1_sig = interface.sign_proposal(&inf_chain1);
    let message: SignedProposalMessage<Header<_>, _> = SignedProposalMessage {
        message: inf_chain1.clone(),
        signature: inf_chain1_sig,
    };
    assert!(interface.verify_proposal(&message, None).is_err());

    temp_producer2
        .push(second_to_last1)
        .expect("Pushing inferior micro block should succeed.");
    assert!(interface.verify_proposal(&message, None).is_err());

    temp_producer2
        .push(second_to_last2)
        .expect("Pushing inferior micro block should succeed.");

    // After the last micro block is pushed the proposal should verify
    // The calculated body should match the one given by block production itself.
    let body = interface
        .verify_proposal(&message, None)
        .expect("Verification must succeed.");
    assert_eq!(inf_proposal1.body.clone().expect(""), body.0);
}
