use ark_groth16::Proof;
use ark_mnt6_753::MNT6_753;

#[cfg(feature = "zkp-prover")]
mod prove;
mod verify;

/// This the main struct for the nano-zkp crate. It provides methods to setup (create the
/// proving and verifying keys), create proofs and verify proofs for the nano sync circuit.
pub struct NanoZKP;

/// This is the proof type for the NanoZKP. It is just an alias, for convenience.
pub type NanoProof = Proof<MNT6_753>;
