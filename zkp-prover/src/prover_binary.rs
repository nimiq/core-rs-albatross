use std::io::{self, BufReader, BufWriter};

use crate::proof_utils::generate_new_proof;
use crate::types::{ProofInput, PROOF_GENERATION_OUTPUT_DELIMITER};
use ark_serialize::Write;
use beserial::{Deserialize, Serialize, SerializingError};

use crate::types::ZKProofGenerationError;

pub async fn prover_main() -> Result<(), SerializingError> {
    // Read proof input from stdin.
    let mut stdin = BufReader::new(io::stdin());
    let proof_input: Result<ProofInput, _> = Deserialize::deserialize(&mut stdin);

    // Then generate proof.
    let result = match proof_input {
        Ok(proof_input) => generate_new_proof(
            proof_input.block,
            proof_input.latest_pks,
            proof_input.latest_header_hash.into(),
            proof_input.previous_proof,
            proof_input.genesis_state,
        ),
        Err(e) => Err(ZKProofGenerationError::from(e)),
    };
    log::info!("Finished proof generation with result {:?}", result);

    // Then print delimiter followed by the serialized result.
    let mut stdout = BufWriter::new(io::stdout());
    stdout.write_all(&PROOF_GENERATION_OUTPUT_DELIMITER)?;

    Serialize::serialize(&result, &mut stdout)?;
    Ok(())
}
