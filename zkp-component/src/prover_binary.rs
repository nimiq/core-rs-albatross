use std::io::{self, BufReader, BufWriter};

use ark_serialize::Write;
use beserial::{Deserialize, Serialize, SerializingError};

use crate::{
    proof_gen_utils::generate_new_proof,
    types::{ProofInput, ZKProofGenerationError, PROOF_GENERATION_OUTPUT_DELIMITER},
};

pub async fn prover_main() -> Result<(), SerializingError> {
    // Read proof input from stdin.
    let mut stdin = BufReader::new(io::stdin());
    let proof_input: Result<ProofInput, _> = Deserialize::deserialize(&mut stdin);

    log::info!(
        "Starting proof generation for block {:?}",
        proof_input.as_ref().map(|input| &input.final_block)
    );

    // Then generate proof.
    let result = match proof_input {
        Ok(proof_input) => generate_new_proof(
            proof_input.previous_block,
            proof_input.previous_proof,
            proof_input.final_block,
            proof_input.genesis_header_hash,
            &proof_input.prover_keys_path,
        ),
        Err(e) => Err(ZKProofGenerationError::from(e)),
    };
    log::info!("Finished proof generation with result {:?}", result);

    // Then print delimiter followed by the serialized result.
    let mut stdout = BufWriter::new(io::stdout());
    stdout.write_all(&PROOF_GENERATION_OUTPUT_DELIMITER)?;

    Serialize::serialize(&result, &mut stdout)?;
    stdout.flush()?;

    Ok(())
}
