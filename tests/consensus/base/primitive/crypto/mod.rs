use core_rs::consensus::base::primitive::crypto::{PrivateKey,PublicKey,Signature,KeyPair};

#[test]
fn it_can_verify_created_signature() {
    let key_pair = KeyPair::generate();
    let data = b"test";
    let signature = key_pair.sign(data);
}
