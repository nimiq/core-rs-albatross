use std::{
    fmt::Debug,
    sync::Arc,
    task::Poll,
    time::{Duration, SystemTime},
};

use futures::{future::poll_fn, stream, FutureExt, StreamExt};
use nimiq_account::{Account, StakingContractStoreWrite, TransactionLog};
use nimiq_block::{Block, BlockError, MacroBlock, MacroHeader};
use nimiq_blockchain::Blockchain;
use nimiq_blockchain_interface::{AbstractBlockchain, PushError, PushResult};
use nimiq_bls::KeyPair as BlsKeyPair;
use nimiq_database::traits::WriteTransaction;
use nimiq_genesis::NetworkInfo;
use nimiq_genesis_builder::GenesisInfo;
use nimiq_hash::{Blake2sHash, Hash};
use nimiq_keys::{Address, KeyPair, SecureGenerate};
use nimiq_network_libp2p::Network;
use nimiq_network_mock::{MockHub, MockNetwork};
use nimiq_primitives::{coin::Coin, key_nibbles::KeyNibbles, networks::NetworkId, policy::Policy};
use nimiq_tendermint::{
    ProposalMessage, Protocol, ProtocolError, Return, SignedProposalMessage,
    State as TendermintState, Step, Tendermint,
};
use nimiq_test_log::test;
use nimiq_test_utils::{
    bitset::with_trailing_zero_limbs,
    block_production::TemporaryBlockProducer,
    blockchain::{
        fill_micro_blocks, produce_macro_blocks, signing_key, validator_address, voting_key,
    },
    test_network::TestNetwork,
    test_rng,
    validator::build_validator,
};
use nimiq_time::timeout;
use nimiq_transaction::account::staking_contract::SignalDataUpdate;
use nimiq_utils::time::systemtime_to_timestamp;
use nimiq_validator::{
    aggregation::tendermint::{proposal::Header, state::MacroState},
    tendermint::TendermintProtocol,
    validator::Validator,
};
use nimiq_validator_network::network_impl::ValidatorNetworkImpl;
use parking_lot::Mutex;
use tracing_core::{
    field::{Field, Visit},
    Event, Subscriber,
};
use tracing_subscriber::{
    fmt,
    layer::{Context, SubscriberExt},
    util::SubscriberInitExt,
    Layer,
};

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
    let nw = Network::build_network(0, Default::default(), &mut Some(hub)).await;
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
    let nw = Network::build_network(0, Default::default(), &mut Some(hub)).await;
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
    let nw = Network::build_network(0, Default::default(), &mut Some(hub)).await;
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
    let nw = Network::build_network(0, Default::default(), &mut Some(hub)).await;
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

/// A macro block's timestamp must be at least one block separation time after its predecessor,
/// so a batch does not lose a second at the macro block.
#[test(tokio::test)]
async fn it_ensures_macro_block_observes_block_separation_time() {
    // Anchor genesis at the current whole second (genesis timestamps are whole seconds) so the
    // head's deterministic timestamps sit near real time. With the fixed historical unit-genesis
    // timestamp the head would be far in the past, the proposer would stamp `now()` either way, and
    // the regression would stay hidden.
    let genesis_timestamp = systemtime_to_timestamp(SystemTime::now()) / 1000 * 1000;
    let temp_producer = TemporaryBlockProducer::new_with_genesis_timestamp(genesis_timestamp);

    // Produce every micro block of the batch, leaving the head just before the macro block.
    for _ in 0..Policy::blocks_per_batch() - 1 {
        temp_producer.next_block(vec![], false);
    }

    let blockchain = Arc::clone(&temp_producer.blockchain);
    let (predecessor_timestamp, macro_block_number, current_validators) = {
        let bc = blockchain.read();
        (
            bc.timestamp(),
            bc.block_number() + 1,
            bc.current_validators().unwrap().clone(),
        )
    };

    // The proposal path never touches the network, so a mock network suffices.
    let hub = MockHub::default();
    let nw = MockNetwork::build_network(0, Default::default(), &mut Some(hub)).await;
    let val_net = Arc::new(ValidatorNetworkImpl::new(nw));
    let interface = TendermintProtocol::new(
        Arc::clone(&blockchain),
        val_net,
        temp_producer.producer.clone(),
        current_validators,
        0,
        NetworkId::UnitAlbatross,
        macro_block_number,
    );

    let (message, body) = interface
        .create_proposal(0)
        .expect("Should have created proposal");
    let macro_header = message.proposal.0.clone();

    // Guard against a slow run: if wall-clock has already reached the head, the old `max(now, head)`
    // would satisfy the assertion below too and the test would pass on regressed code.
    assert!(
        blockchain.read().time.now() < predecessor_timestamp + Policy::BLOCK_SEPARATION_TIME,
        "test setup outran the head timestamp; cannot observe the regression",
    );

    // The proposer gives the macro block its own slot, one separation time past the predecessor.
    assert!(
        macro_header.timestamp >= predecessor_timestamp + Policy::BLOCK_SEPARATION_TIME,
        "macro block timestamp {} must be at least one block separation time ({} ms) after its \
         predecessor's timestamp {}",
        macro_header.timestamp,
        Policy::BLOCK_SEPARATION_TIME,
        predecessor_timestamp,
    );

    // The stamp is ahead of the local clock but within the drift allowance, so verifying the
    // proposal must still succeed.
    let macro_block = MacroBlock {
        header: macro_header,
        body: Some(body.0),
        justification: None,
    };
    blockchain
        .read()
        .verify_macro_block_proposal(macro_block, 0, None)
        .expect("a proposal within the drift allowance must verify");
}

/// A proposal with a padded punished set verifies like its canonical twin but is not endorsed: the
/// node keeps it, follows a polka on it and only withholds its own prevote, so the refusal can cost
/// the proposer a round but never stall the height. Rejecting it instead would leave an unpatched
/// 2f+1 locked on a value the patched validators never precommit.
#[test(tokio::test)]
async fn it_does_not_endorse_a_padded_punished_set() {
    let temp_producer = TemporaryBlockProducer::default();
    fill_micro_blocks(&temp_producer.producer, &temp_producer.blockchain);
    let blockchain = Arc::clone(&temp_producer.blockchain);
    let (macro_block_number, current_validators) = {
        let bc = blockchain.read();
        (
            bc.block_number() + 1,
            bc.current_validators().unwrap().clone(),
        )
    };

    let hub = MockHub::default();
    let nw = MockNetwork::build_network(0, Default::default(), &mut Some(hub)).await;
    let val_net = Arc::new(ValidatorNetworkImpl::new(nw));
    let interface = TendermintProtocol::new(
        Arc::clone(&blockchain),
        val_net,
        temp_producer.producer.clone(),
        current_validators,
        0,
        NetworkId::UnitAlbatross,
        macro_block_number,
    );

    let (message, _body) = interface
        .create_proposal(0)
        .expect("Should have created proposal");
    let canonical = message.proposal.0;
    assert!(canonical.next_batch_initial_punished_set.is_canonical());

    // What a malicious proposer gossips: the same block with its punished set padded.
    let mut padded = canonical.clone();
    padded.next_batch_initial_punished_set =
        with_trailing_zero_limbs(&canonical.next_batch_initial_punished_set, 64);
    padded.cached_hash = None;
    assert!(!padded.next_batch_initial_punished_set.is_canonical());

    // Both are valid proposals.
    for header in [&canonical, &padded] {
        let message = ProposalMessage {
            round: 0,
            valid_round: None,
            proposal: Header(header.clone(), None),
        };
        let signature = interface.sign_proposal(&message);
        assert!(interface
            .verify_proposal(&SignedProposalMessage { message, signature }, None)
            .is_ok());
    }

    // Only the canonical one gets this node's prevote.
    assert!(interface.endorses_proposal(&Header(canonical, None)));
    assert!(!interface.endorses_proposal(&Header(padded, None)));
}

/// Runs the Tendermint state machine on the real protocol as a validator awaiting the proposal of
/// round 0, hands it `header` as that proposal and returns the prevote it casts.
async fn round_0_prevote(
    temp_producer: &TemporaryBlockProducer,
    header: MacroHeader,
) -> Option<Blake2sHash> {
    let blockchain = Arc::clone(&temp_producer.blockchain);
    let (macro_block_number, current_validators) = {
        let bc = blockchain.read();
        (
            bc.block_number() + 1,
            bc.current_validators().unwrap().clone(),
        )
    };

    let hub = MockHub::default();
    let nw = MockNetwork::build_network(0, Default::default(), &mut Some(hub)).await;
    let interface = TendermintProtocol::new(
        blockchain,
        Arc::new(ValidatorNetworkImpl::new(nw)),
        temp_producer.producer.clone(),
        current_validators,
        // The only validator has band 0 and proposes every round, any other band awaits it.
        1,
        NetworkId::UnitAlbatross,
        macro_block_number,
    );
    assert!(!interface.is_proposer(0).unwrap());

    let message = ProposalMessage {
        round: 0,
        valid_round: None,
        proposal: Header(header, None),
    };
    let signature = interface.sign_proposal(&message);
    let mut tendermint = Tendermint::new(
        interface,
        None,
        stream::iter([SignedProposalMessage { message, signature }])
            .chain(stream::pending())
            .boxed(),
        stream::pending().boxed(),
    );

    // Padded or not, the proposal is valid.
    assert!(matches!(
        tendermint.next().await,
        Some(Return::ProposalAccepted(_))
    ));
    // The next state carries the prevote, the aggregation signing it would only start after it.
    let Some(Return::Update(state)) = tendermint.next().await else {
        panic!("Expected the state carrying the prevote");
    };
    state.votes[&(0, Step::Prevote)].clone()
}

/// The same policy as the validator's state machine applies it: a canonical proposal gets the
/// prevote and its padded twin a nil one, at a checkpoint and at an election block, whose header
/// nodes keep forever.
#[test(tokio::test)]
async fn it_prevotes_nil_for_a_padded_punished_set() {
    let checkpoint = TemporaryBlockProducer::default();
    fill_micro_blocks(&checkpoint.producer, &checkpoint.blockchain);

    let election = TemporaryBlockProducer::default();
    produce_macro_blocks(
        &election.producer,
        &election.blockchain,
        Policy::batches_per_epoch() as usize - 1,
    );
    fill_micro_blocks(&election.producer, &election.blockchain);

    for (temp_producer, is_election) in [(checkpoint, false), (election, true)] {
        let canonical = {
            let blockchain = temp_producer.blockchain.read();
            temp_producer
                .producer
                .next_macro_block_proposal(
                    &blockchain,
                    blockchain.timestamp() + Policy::BLOCK_SEPARATION_TIME,
                    0,
                    vec![],
                    None,
                )
                .unwrap()
                .header
        };
        assert_eq!(canonical.is_election(), is_election);

        let mut padded = canonical.clone();
        padded.next_batch_initial_punished_set =
            with_trailing_zero_limbs(&canonical.next_batch_initial_punished_set, 64);
        padded.cached_hash = None;

        let canonical_hash: Blake2sHash = Hash::hash(&canonical);
        assert_eq!(
            round_0_prevote(&temp_producer, canonical).await,
            Some(canonical_hash)
        );
        assert_eq!(round_0_prevote(&temp_producer, padded).await, None);
    }
}

/// Runs `action` on the first log event with the given message, from within the logging call.
struct RunOnLog {
    message: &'static str,
    action: Mutex<Option<Box<dyn FnOnce() + Send>>>,
}

impl<S: Subscriber> Layer<S> for RunOnLog {
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let mut visitor = MessageEq {
            expected: self.message,
            matches: false,
        };
        event.record(&mut visitor);
        if !visitor.matches {
            return;
        }
        // Release the lock before running the action, as the action may log itself.
        let action = self.action.lock().take();
        if let Some(action) = action {
            action();
        }
    }
}

struct MessageEq {
    expected: &'static str,
    matches: bool,
}

impl Visit for MessageEq {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.matches = format!("{value:?}") == self.expected;
        }
    }
}

/// Polls the validator until macro block `block_number` is the head of its chain.
async fn finish_macro_block(
    validator: &mut Validator<ValidatorNetworkImpl<MockNetwork>>,
    block_number: u32,
) -> MacroBlock {
    let blockchain = Arc::clone(&validator.blockchain);
    poll_fn(|cx| {
        let _ = validator.poll_unpin(cx);
        match blockchain.read().head() {
            Block::Macro(block) if block.block_number() == block_number => {
                Poll::Ready(block.clone())
            }
            _ => Poll::Pending,
        }
    })
    .await
}

fn tendermint_state(
    macro_state: Option<MacroState>,
    block_number: u32,
) -> Option<TendermintState<TendermintProtocol<ValidatorNetworkImpl<MockNetwork>>>> {
    macro_state?.into_tendermint_state(block_number)
}

/// The only validator is at head H-1 when a competing skip block for H-2 becomes the head, just as
/// its Tendermint instance for macro block H reaches a prevote polka. It must still finish H, from a
/// persisted precommit and lock, and without changing any vote it had persisted before.
#[test(tokio::test)]
async fn it_finishes_macro_block_from_persisted_state_after_rebranch() {
    let producer = TemporaryBlockProducer::new();
    let network_info = NetworkInfo::from_network_id(NetworkId::UnitAlbatross);
    let genesis = GenesisInfo {
        block: network_info.genesis_block(),
        hash: network_info.genesis_hash().clone(),
        accounts: network_info.genesis_accounts(),
    };
    let (mut validator, mut consensus) = build_validator::<MockNetwork>(
        0,
        validator_address(),
        false,
        signing_key(),
        voting_key(),
        signing_key(),
        genesis,
        &mut Some(MockHub::default()),
    )
    .await;
    let blockchain = Arc::clone(&validator.blockchain);
    let push = |block| Blockchain::push(blockchain.upgradable_read(), block).unwrap();

    // Head at H-1, with a competing skip block for H-2 kept aside.
    let macro_block_number = Policy::macro_block_after(blockchain.read().block_number());
    while producer.blockchain.read().block_number() < macro_block_number - 3 {
        push(producer.next_block(vec![], false));
    }
    let skip_block = producer.next_block_no_push(vec![], true);
    push(producer.next_block(vec![], false));
    push(producer.next_block(vec![], false));

    // The skip block becomes the head the moment Tendermint sees the prevote polka.
    let rebranch = Arc::new(Mutex::new(None));
    let state_at_polka = Arc::new(Mutex::new(None));
    let action = {
        let blockchain = Arc::clone(&blockchain);
        let macro_state = Arc::clone(validator.macro_state());
        let rebranch = Arc::clone(&rebranch);
        let state_at_polka = Arc::clone(&state_at_polka);
        move || {
            *state_at_polka.lock() = macro_state.read().clone();
            *rebranch.lock() = Some(Blockchain::push(blockchain.upgradable_read(), skip_block));
        }
    };
    let _log_guard = tracing_subscriber::registry()
        .with(RunOnLog {
            message: "Aggregation resulted in Block polka",
            action: Mutex::new(Some(Box::new(action))),
        })
        .with(
            fmt::layer()
                .with_test_writer()
                .with_filter(nimiq_test_log::log_filter()),
        )
        .set_default();

    consensus.force_established();
    let decision = timeout(
        Duration::from_secs(20),
        finish_macro_block(&mut validator, macro_block_number),
    )
    .await
    .expect("the validator must finish the macro block");
    assert_eq!(
        *rebranch.lock(),
        Some(Ok(PushResult::Rebranched)),
        "the skip block must become the head while the macro block is produced",
    );

    // As the only validator, its own precommit decides H, so that precommit must be persisted.
    let persisted = tendermint_state(validator.macro_state().read().clone(), macro_block_number)
        .expect("a Tendermint state for the finished macro block must be persisted");
    let round = decision.justification.as_ref().unwrap().round;
    let proposal_hash = Hash::hash::<Blake2sHash>(&decision.header);
    assert_eq!(
        persisted.votes.get(&(round, Step::Precommit)),
        Some(&Some(proposal_hash.clone())),
        "finished {decision} without a persisted precommit, persisted votes: {:?}",
        persisted.votes,
    );
    assert_eq!(persisted.locked, Some((round, proposal_hash)));

    // Votes persisted before the rebranch may have left the node, so they must not change.
    let votes_at_polka = tendermint_state(state_at_polka.lock().take(), macro_block_number)
        .expect("the prevote must be persisted when the polka forms")
        .votes;
    for (id, vote) in &votes_at_polka {
        assert_eq!(persisted.votes.get(id), Some(vote), "vote {id:?} changed");
    }
}
