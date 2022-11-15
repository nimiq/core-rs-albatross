use nimiq_test_log::test;
use nimiq_test_utils::zkp_test_data::zkp_test_exe;
use nimiq_zkp_component::{
    proof_utils::launch_generate_new_proof,
    types::{ProofInput, ZKProofGenerationError},
};
use tokio::sync::broadcast;

#[test]
fn can_locate_prover_binary() {
    zkp_test_exe();
}

#[test(tokio::test)]
async fn can_launch_process_and_parse_output() {
    let (_send, recv) = broadcast::channel(1);
    let proof_input: ProofInput = Default::default();

    let result = launch_generate_new_proof(recv, proof_input, Some(zkp_test_exe())).await;

    assert_eq!(result, Err(ZKProofGenerationError::InvalidBlock));
}

#[test(tokio::test)]
async fn can_launch_process_and_kill() {
    let (send, recv) = broadcast::channel(1);
    let proof_input: ProofInput = Default::default();

    let result = tokio::spawn(launch_generate_new_proof(
        recv,
        proof_input,
        Some(zkp_test_exe()),
    ));
    send.send(()).unwrap();

    assert_eq!(
        result.await.unwrap(),
        Err(ZKProofGenerationError::ChannelError)
    );
}
