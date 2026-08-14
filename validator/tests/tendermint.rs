use std::sync::Arc;

use nimiq_account::{Account, StakingContractStoreWrite, TransactionLog};
use nimiq_block::BlockError;
use nimiq_blockchain_interface::{AbstractBlockchain, PushError};
use nimiq_bls::KeyPair as BlsKeyPair;
use nimiq_database::traits::WriteTransaction;
use nimiq_keys::{Address, KeyPair, SecureGenerate};
use nimiq_network_libp2p::Network;
use nimiq_network_mock::MockHub;
use nimiq_primitives::{coin::Coin, key_nibbles::KeyNibbles, networks::NetworkId, policy::Policy};
use nimiq_tendermint::{ProposalMessage, Protocol, ProtocolError, SignedProposalMessage};
use nimiq_test_log::test;
use nimiq_test_utils::{
    block_production::TemporaryBlockProducer, test_network::TestNetwork, test_rng,
};
use nimiq_transaction::account::staking_contract::SignalDataUpdate;
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
        temp_producer1
            .producer
            .next_macro_block_proposal(
                &b,
                b.timestamp() + Policy::BLOCK_SEPARATION_TIME,
                0,
                vec![],
                None,
            )
            .unwrap()
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
        temp_producer1
            .producer
            .next_macro_block_proposal(
                &b,
                b.timestamp() + Policy::BLOCK_SEPARATION_TIME,
                0,
                vec![],
                None,
            )
            .unwrap()
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
        temp_producer1
            .producer
            .next_macro_block_proposal(
                &b,
                b.timestamp() + Policy::BLOCK_SEPARATION_TIME,
                0,
                vec![],
                None,
            )
            .unwrap()
    };

    // blockchain2 here has not seen any of the inferior micro blocks nor any of the proposals.

    // Create TendermintProtocol for blockchain2
    let current_validators = blockchain1.read().current_validators().unwrap().clone();
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

/// Creates a second validator with the same stake as the first in UnitAlbatross.
fn create_validator(temp_producer: &TemporaryBlockProducer) -> Address {
    let blockchain = Arc::clone(&temp_producer.blockchain);

    let blockchain_rg = blockchain.read();
    let mut staking_contract = blockchain_rg.get_staking_contract();
    let block_number = blockchain_rg.head().block_number();
    let total_stake = staking_contract.balance;
    let current_stake =
        staking_contract.balance - Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT);

    // We create a new account with that balance.
    let mut rng = test_rng(true);
    let validator_key_pair = KeyPair::generate(&mut rng);
    let validator_voting_key_pair = BlsKeyPair::generate(&mut rng);
    let staker_key_pair = KeyPair::generate(&mut rng);

    // Get the deposit value.
    let deposit = Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT);

    let mut raw_txn = blockchain_rg.write_transaction();
    let data_store = blockchain_rg
        .state()
        .accounts
        .data_store(&Policy::STAKING_CONTRACT_ADDRESS);
    let mut txn = (&mut raw_txn).into();
    let mut data_store_write = data_store.write(&mut txn);
    let mut store = StakingContractStoreWrite::new(&mut data_store_write);

    staking_contract
        .create_validator(
            &mut store,
            &Address::from(&validator_key_pair),
            validator_key_pair.public,
            validator_voting_key_pair.public_key.compress(),
            Address::from(&validator_key_pair),
            None,
            deposit,
            None,
            None,
            false,
            block_number,
            &mut TransactionLog::empty(),
        )
        .expect("Failed to create validator");

    staking_contract
        .create_staker(
            &mut store,
            &Address::from(&staker_key_pair),
            current_stake,
            Some(Address::from(&validator_key_pair)),
            Coin::ZERO,
            None,
            &mut TransactionLog::empty(),
        )
        .expect("Failed to create staker");

    assert_eq!(
        staking_contract.balance,
        total_stake.checked_mul(2).unwrap()
    );

    blockchain_rg
        .state()
        .accounts
        .tree
        .put(
            &mut txn,
            &KeyNibbles::from(&Policy::STAKING_CONTRACT_ADDRESS),
            Account::Staking(staking_contract),
        )
        .expect("Failed to put account into tree");

    raw_txn.commit();

    Address::from(&validator_key_pair)
}

/// Signals support for a specific version for a given validator, using the warm-key
/// `set_signal_data` path (the mechanism exercised by the `SetSignalData` transaction).
fn signal_support(
    temp_producer: &TemporaryBlockProducer,
    validator_address: &Address,
    version: u16,
) {
    let blockchain = Arc::clone(&temp_producer.blockchain);

    let blockchain_rg = blockchain.read();
    let mut staking_contract = blockchain_rg.get_staking_contract();
    let data_store = blockchain_rg
        .state()
        .accounts
        .data_store(&Policy::STAKING_CONTRACT_ADDRESS);

    // The signal data is authorized by the validator's warm (signing) key.
    let signer = {
        let read_txn = blockchain_rg.read_transaction();
        Address::from(
            &staking_contract
                .get_validator(&data_store.read(&read_txn), validator_address)
                .expect("Validator should exist")
                .signing_key,
        )
    };

    let mut raw_txn = blockchain_rg.write_transaction();
    let mut txn = (&mut raw_txn).into();
    let mut data_store_write = data_store.write(&mut txn);
    let mut store = StakingContractStoreWrite::new(&mut data_store_write);

    staking_contract
        .set_signal_data(
            &mut store,
            validator_address,
            &signer,
            SignalDataUpdate::Version(version),
            &mut TransactionLog::empty(),
        )
        .expect("Failed to set signal data");

    blockchain_rg
        .state()
        .accounts
        .tree
        .put(
            &mut txn,
            &KeyNibbles::from(&Policy::STAKING_CONTRACT_ADDRESS),
            Account::Staking(staking_contract),
        )
        .expect("Failed to put account into tree");

    raw_txn.commit();
}

#[test(tokio::test)]
async fn it_triggers_version_upgrades() {
    // Move to before the next election block.
    let initial_version = Policy::max_supported_version() - 1;
    let next_version = Policy::max_supported_version();
    let temp_producer = TemporaryBlockProducer::new_with_protocol_version(initial_version);
    for _ in 0..Policy::blocks_per_epoch() - 1 {
        let block = temp_producer.next_block(vec![], false);
        temp_producer
            .push(block)
            .expect("Should be able to push block");
    }

    // Initialize a TendermintProtocol.
    let blockchain = Arc::clone(&temp_producer.blockchain);
    let current_validators = blockchain.read().current_validators().unwrap().clone();
    assert_eq!(
        current_validators.num_validators(),
        1,
        "Test setup assumes one validator"
    );
    assert_eq!(
        blockchain.read().state().current_version(),
        initial_version,
        "Test assumes current version is one below the maximum supported version"
    );
    assert_eq!(
        Policy::max_supported_version(),
        next_version,
        "Test assumes max supported version"
    );
    let validator1 = current_validators.validators[0].address.clone();
    let hub = MockHub::default();
    let nw: Arc<Network> = TestNetwork::build_network(0, Default::default(), &mut Some(hub)).await;
    let val_net = Arc::new(ValidatorNetworkImpl::new(nw));
    let interface = TendermintProtocol::new(
        Arc::clone(&blockchain),
        val_net,
        temp_producer.producer.clone(),
        current_validators,
        0,
        NetworkId::UnitAlbatross,
        blockchain.read().head().block_number() + 1,
    );

    // Setup second validator with same stake.
    let validator2 = create_validator(&temp_producer);

    // Case 1: No support.
    let proposal = interface
        .create_proposal(0)
        .expect("Should have created proposal");
    assert_eq!(proposal.0.proposal.0.version, initial_version);

    // Case 2: The elected slots support the upgrade (validator1 holds them all), but only half of
    // the active stake does, so the stake threshold is not reached.
    signal_support(&temp_producer, &validator1, next_version);
    let proposal = interface
        .create_proposal(0)
        .expect("Should have created proposal");
    assert_eq!(proposal.0.proposal.0.version, initial_version);

    // Case 3: Both the elected slots and the active stake support the upgrade.
    signal_support(&temp_producer, &validator2, next_version);

    let proposal = interface
        .create_proposal(0)
        .expect("Should have created proposal");
    assert_eq!(proposal.0.proposal.0.version, next_version);
}

#[test(tokio::test)]
async fn it_aborts_proposal_creation_with_incomplete_accounts() {
    // Move to before the next election block, with an upgrade to the next version pending.
    let initial_version = Policy::max_supported_version() - 1;
    let temp_producer = TemporaryBlockProducer::new_with_protocol_version(initial_version);
    for _ in 0..Policy::blocks_per_epoch() - 1 {
        let block = temp_producer.next_block(vec![], false);
        temp_producer
            .push(block)
            .expect("Should be able to push block");
    }

    // Initialize a TendermintProtocol.
    let blockchain = Arc::clone(&temp_producer.blockchain);
    let current_validators = blockchain.read().current_validators().unwrap().clone();
    let hub = MockHub::default();
    let nw: Arc<Network> = TestNetwork::build_network(0, Default::default(), &mut Some(hub)).await;
    let val_net = Arc::new(ValidatorNetworkImpl::new(nw));
    let interface = TendermintProtocol::new(
        Arc::clone(&blockchain),
        val_net,
        temp_producer.producer.clone(),
        current_validators,
        0,
        NetworkId::UnitAlbatross,
        blockchain.read().head().block_number() + 1,
    );

    // Make the accounts trie incomplete.
    {
        let blockchain_rg = blockchain.read();
        let mut raw_txn = blockchain_rg.write_transaction();
        blockchain_rg
            .state()
            .accounts
            .reinitialize_as_incomplete(&mut (&mut raw_txn).into());
        raw_txn.commit();
    }

    // The version-upgrade support check must abort the proposal instead of panicking.
    assert!(matches!(
        interface.create_proposal(0),
        Err(ProtocolError::Abort)
    ));
}

#[test(tokio::test)]
async fn it_does_not_trigger_version_upgrades_on_checkpoint_blocks() {
    let initial_version = Policy::max_supported_version() - 1;
    let next_version = Policy::max_supported_version();

    // Move to just before the first checkpoint (non-election) macro block
    let temp_producer = TemporaryBlockProducer::new_with_protocol_version(initial_version);
    for _ in 0..Policy::blocks_per_batch() - 1 {
        let block = temp_producer.next_block(vec![], false);
        temp_producer
            .push(block)
            .expect("Should be able to push block");
    }

    // Initialize a TendermintProtocol.
    let blockchain = Arc::clone(&temp_producer.blockchain);
    let current_validators = blockchain.read().current_validators().unwrap().clone();
    let proposal_height = blockchain.read().head().block_number() + 1;
    assert!(
        Policy::is_macro_block_at(proposal_height)
            && !Policy::is_election_block_at(proposal_height),
        "Test setup assumes the next macro block is a checkpoint block"
    );
    assert_eq!(
        current_validators.num_validators(),
        1,
        "Test setup assumes one validator"
    );
    assert_eq!(
        blockchain.read().state().current_version(),
        initial_version,
        "Test assumes current version"
    );
    assert_eq!(
        Policy::max_supported_version(),
        next_version,
        "Test assumes max supported version"
    );
    let validator1 = current_validators.validators[0].address.clone();
    let hub = MockHub::default();
    let nw: Arc<Network> = TestNetwork::build_network(0, Default::default(), &mut Some(hub)).await;
    let val_net = Arc::new(ValidatorNetworkImpl::new(nw));
    let interface = TendermintProtocol::new(
        Arc::clone(&blockchain),
        val_net,
        temp_producer.producer.clone(),
        current_validators,
        0,
        NetworkId::UnitAlbatross,
        proposal_height,
    );

    // Setup second validator with same stake and signal full support from both
    let validator2 = create_validator(&temp_producer);
    signal_support(&temp_producer, &validator1, next_version);
    signal_support(&temp_producer, &validator2, next_version);

    // Even with both thresholds reached, a checkpoint block must keep the current version,
    // since the network only accepts version increases on election blocks
    let proposal = interface
        .create_proposal(0)
        .expect("Should have created proposal");
    assert_eq!(proposal.0.proposal.0.version, initial_version);
}

/// The validation side must reject an election block that bumps the version without sufficient
/// support, mirroring the proposer-side check (otherwise a malicious proposer could force the
/// upgrade and deactivate most of the active validators).
#[test(tokio::test)]
async fn it_rejects_unsupported_version_upgrade_blocks() {
    let initial_version = Policy::max_supported_version() - 1;
    let next_version = Policy::max_supported_version();

    // Move to before the next election block.
    let temp_producer = TemporaryBlockProducer::new_with_protocol_version(initial_version);
    for _ in 0..Policy::blocks_per_epoch() - 1 {
        let block = temp_producer.next_block(vec![], false);
        temp_producer
            .push(block)
            .expect("Should be able to push block");
    }

    let blockchain = Arc::clone(&temp_producer.blockchain);
    let validator1 = blockchain.read().current_validators().unwrap().validators[0]
        .address
        .clone();

    // Add a second validator with the same stake and let only the elected validator1 signal
    // support. This yields full slot support but only half of the active stake, below the
    // `UPGRADE_MIN_SUPPORT` threshold, so the upgrade is not supported.
    let _validator2 = create_validator(&temp_producer);
    signal_support(&temp_producer, &validator1, next_version);

    // Forcefully craft an election block that bumps the version despite the missing support.
    let proposal = {
        let bc = blockchain.read();
        temp_producer
            .producer
            .next_macro_block_proposal(
                &bc,
                bc.timestamp() + Policy::BLOCK_SEPARATION_TIME,
                0,
                vec![],
                Some(next_version),
            )
            .expect("Should have created proposal")
    };
    assert!(proposal.is_election());
    assert_eq!(proposal.header.version, next_version);

    // The network must reject the proposal because the upgrade lacks sufficient support.
    let result = blockchain
        .read()
        .verify_macro_block_proposal(proposal, 0, None);
    assert!(
        matches!(
            result,
            Err(PushError::InvalidBlock(BlockError::InvalidVersionUpgrade))
        ),
        "Expected InvalidVersionUpgrade, got {result:?}"
    );
}
