use beserial::{Deserialize, Serialize};
use nimiq_bls::bls12_381::{Signature, PublicKey};
use crate::BlockHeader;


#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ForkProof {
    pub header1: BlockHeader,
    pub header2: BlockHeader,
    pub justification1: Signature,
    pub justification2: Signature,
}

impl ForkProof {
    pub fn pre_verify(&self) -> Option<(/* block_number */ u32, /* view_number */ u32)> {
        let block_number = self.header1.block_number();
        let view_number = self.header2.view_number();
        if block_number == self.header2.block_number() && view_number == self.header2.view_number() {
            Some((block_number, view_number))
        }
        else {
            None
        }
    }

    pub fn verify(&self, public_key: &PublicKey) -> bool {
        public_key.verify(&self.header1, &self.justification1)
            && public_key.verify(&self.header2, &self.justification2)
    }
}
