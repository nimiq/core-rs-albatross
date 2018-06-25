use core_rs::consensus::base::primitive::crypto::{PrivateKey,PublicKey,Signature,KeyPair};
use core_rs::consensus::base::primitive::crypto::multisig::{RandomSecret,Commitment,PartialSignature};
use curve25519_dalek::scalar::Scalar;
use curve25519_dalek::edwards::EdwardsPoint;

struct TestVector {
    priv_keys: Vec<PrivateKey>,
    pub_keys: Vec<PublicKey>,
    pub_keys_hash: [u8; 64],
    delinearized_priv_keys: Vec<Scalar>,
    delinearized_pub_keys: Vec<EdwardsPoint>,
    secrets: Vec<RandomSecret>,
    commitments: Vec<Commitment>,
    agg_pub_key: PublicKey,
    agg_commitment: Commitment,
    partial_signatures: Vec<PartialSignature>,
    agg_signature: PartialSignature,
    signature: Signature,
    message: [u8]
}

#[test]
fn it_can_verify_created_signature() {
    let key_pair = KeyPair::generate();
    let data = b"test";
    let signature = key_pair.sign(data);
}
