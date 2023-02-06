use ark_groth16::Proof;
use ark_mnt6_753::MNT6_753;

#[cfg(feature = "zkp-prover")]
pub mod prove;
pub mod verify;

/// This is the proof type for the NanoZKP. It is just an alias, for convenience.
pub type NanoProof = Proof<MNT6_753>;
