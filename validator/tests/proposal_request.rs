use std::{collections::BTreeMap, time::Duration};

use nimiq_block::MacroHeader;
use nimiq_blockchain_interface::AbstractBlockchain;
use nimiq_database::mdbx::MdbxDatabase;
use nimiq_hash::Blake2sHash;
use nimiq_keys::{Ed25519Signature, KeyPair, SecureGenerate};
use nimiq_network_mock::{MockHub, MockNetwork};
use nimiq_serde::Serialize as _;
use nimiq_tendermint::State as TendermintState;
use nimiq_test_log::test;
use nimiq_test_utils::validator::{build_validators, seeded_rng};
use nimiq_validator::{
    aggregation::tendermint::{
        proposal::{Header, RequestProposal, SignedProposal},
        state::MacroState,
    },
    tendermint::TendermintProtocol,
};
use nimiq_validator_network::{network_impl::ValidatorNetworkImpl, ValidatorNetwork};

type ValNet = ValidatorNetworkImpl<MockNetwork>;

/// Polls the validator network until the peer ID for `validator_id` has been
/// resolved through the (mock) DHT. Resolution is performed in a spawned task,
/// so we give it a few iterations to settle.
async fn resolve_peer_id(network: &ValNet, validator_id: u16) {
    for _ in 0..50 {
        if network.get_peer_id(validator_id).is_some() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("Failed to resolve peer ID for validator {validator_id}");
}

/// Sends a `RequestProposal` through the validator network, retrying a few times
/// to absorb DHT-resolution races. Returns `Ok(response)` if the request reached
/// a handler, or `Err(debug_string)` if it never got through.
async fn request_proposal_with_retry(
    network: &ValNet,
    request: &RequestProposal,
    validator_id: u16,
) -> Result<Option<SignedProposal>, String> {
    let mut last_err = String::from("request was never attempted");
    for _ in 0..20 {
        match network
            .request::<RequestProposal>(request.clone(), validator_id)
            .await
        {
            Ok(response) => return Ok(response),
            Err(error) => {
                last_err = format!("{error:?}");
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
    }
    Err(last_err)
}

/// Builds a `MacroState` that knows exactly one proposal, identified by
/// `(block_number, round, proposal_hash)`.
fn macro_state_with_proposal(
    block_number: u32,
    round: u32,
    proposal_hash: Blake2sHash,
    header: MacroHeader,
    signer: u16,
) -> MacroState {
    let mut state = TendermintState::<TendermintProtocol<ValNet>>::default();
    state.current_round = round;
    state
        .known_proposals
        .insert(proposal_hash.clone(), Header(header, None));

    let mut round_map: BTreeMap<Blake2sHash, (Option<u32>, (Ed25519Signature, u16))> =
        BTreeMap::new();
    round_map.insert(proposal_hash, (None, (Ed25519Signature::default(), signer)));
    state.round_proposals.insert(round, round_map);

    MacroState::from_tendermint_state::<ValNet>(block_number, state)
}

/// Regression test for the `RequestProposal` TYPE_ID mismatch between the
/// request sender and the request handler.
///
/// `TendermintProtocol::request_proposal` sends proposal requests through the
/// *validator* network (via `SingleResponseRequester`). The validator network
/// wraps every request in a `ValidatorMessage<RequestProposal>`, which uses
/// wire `TYPE_ID = 10_000 + 199 = 10199`.
///
/// The receiving side, `Validator::init_network_request_receivers`, must
/// therefore register its handler on the validator network as well, so that it
/// listens on the same `TYPE_ID`.
///
/// Before the fix the handler was registered on the *raw* network
/// (`network.receive_requests::<RequestProposal>()`, `TYPE_ID 199`), so no
/// receiver existed for the `TYPE_ID 10199` that the request is actually sent
/// with. As a result, a validator that missed a proposal via gossip could not
/// recover it through a direct request, forcing unnecessary Tendermint round
/// changes.
///
/// This test drives the real wiring end-to-end. It builds two validators (whose
/// constructors run `init_network_request_receivers`) and sends a
/// `RequestProposal` from one to the other through the validator network — the
/// exact same path `request_proposal` uses. It checks both outcomes:
///
/// 1. When the responder does **not** know the requested proposal, it must
///    answer with `None`.
/// 2. After the proposal is made known to the responder, the same request must
///    return exactly that proposal.
///
/// Both checks require the request to *reach the handler in the first place*,
/// which is precisely what the TYPE_ID mismatch breaks:
///
/// * With the bug, the request is never delivered to a handler and the call
///   errors out (no receiver for `TYPE_ID 10199`) — this test fails.
/// * With the fix, the request reaches the handler and both the `None` and the
///   `Some(proposal)` answers are observed — this test passes.
#[test(tokio::test)]
async fn request_proposal_reaches_validator_handler() {
    let hub = MockHub::default();
    let env =
        MdbxDatabase::new_volatile(Default::default()).expect("Could not open a volatile database");

    // Two connected validators. Their constructors wire up the proposal request
    // handler via `Validator::init_network_request_receivers`.
    let validators = build_validators::<MockNetwork>(env, &[1u64, 2u64], &mut Some(hub)).await;

    // The set of elected validators, used to map validator IDs (slot bands) to
    // addresses.
    let elected = validators[0]
        .blockchain
        .read()
        .current_validators()
        .expect("Blockchain should have elected validators")
        .clone();

    // Replicate the validator-network address resolution setup that a running
    // validator performs in `init_epoch` / `set_public_key`. `build_validators`
    // does not spawn the validators themselves, so this is not done for us.
    for validator in &validators {
        let address = validator.state().read().validator_address.clone();
        let validator_id = elected
            .get_slot_band_by_address(&address)
            .expect("Validator should be elected");
        validator.network.set_validators(&elected);
        validator.network.set_validator_id(Some(validator_id));
        validator
            .network
            .set_public_key(
                &address,
                &KeyPair::generate(&mut seeded_rng(100 + validator_id as u64)),
            )
            .await
            .expect("Publishing the validator record should succeed");
    }

    let responder = &validators[0];
    let requester = &validators[1];

    let responder_id = elected
        .get_slot_band_by_address(&responder.state().read().validator_address.clone())
        .unwrap();
    let requester_id = elected
        .get_slot_band_by_address(&requester.state().read().validator_address.clone())
        .unwrap();

    // Make sure both sides can resolve each other's peer ID (DHT lookups are
    // async). The requester needs the responder's peer ID to send the request;
    // the responder needs the requester's peer ID to accept it (sender check in
    // `ValidatorNetwork::receive_requests`).
    resolve_peer_id(&requester.network, responder_id).await;
    resolve_peer_id(&responder.network, requester_id).await;

    // The proposal we are going to ask for.
    let block_number = 42;
    let round = 3;
    let signer = 7;
    let proposal_hash = Blake2sHash::from([0xab; 32]);
    let header = MacroHeader {
        block_number,
        round,
        ..Default::default()
    };
    let request = RequestProposal {
        block_number,
        round_number: round,
        proposal_hash: proposal_hash.clone(),
    };

    // Case 1: the responder does not know the proposal yet (its `MacroState` is
    // empty). A correctly-wired handler must answer with `None`.
    assert!(
        responder.macro_state().read().is_none(),
        "Test precondition: the responder starts without any macro state",
    );
    let response = request_proposal_with_retry(&requester.network, &request, responder_id)
        .await
        .expect(
            "RequestProposal sent over the validator network (TYPE_ID 10199) was not delivered \
             to the handler registered by `Validator::init_network_request_receivers`. The \
             handler is registered on the raw network (TYPE_ID 199), so the wire protocol IDs \
             do not match.",
        );
    assert!(
        response.is_none(),
        "The responder does not know the proposal yet, so it must answer with `None`, got {response:?}",
    );

    // Now make the proposal known to the responder, exactly as a macro block run
    // would populate its cached `MacroState`.
    let macro_state =
        macro_state_with_proposal(block_number, round, proposal_hash.clone(), header, signer);
    let expected = macro_state
        .get_proposal_for(block_number, round, &proposal_hash)
        .expect("The injected macro state must contain the requested proposal");
    *responder.macro_state().write() = Some(macro_state);

    // Case 2: the responder now knows the proposal. The same request must return
    // exactly that proposal.
    let response = request_proposal_with_retry(&requester.network, &request, responder_id)
        .await
        .expect("RequestProposal must reach the handler");
    let signed = response.expect("The responder knows the proposal now, so it must answer `Some`");
    assert_eq!(
        signed.serialize_to_vec(),
        expected.serialize_to_vec(),
        "The responder must return exactly the proposal it knows about",
    );
}
