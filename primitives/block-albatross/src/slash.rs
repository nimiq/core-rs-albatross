use beserial::{Deserialize, Serialize};
use nimiq_bls::bls12_381::Signature;
use crate::micro_block::MicroHeader;


#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SlashInherent {
    pub header1: MicroHeader,
    pub justification1: Signature,
    pub header2: MicroHeader,
    pub justification2: Signature,
}
