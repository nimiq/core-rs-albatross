use beserial::{Serialize, Deserialize};
use super::primitive::hash::Blake2bHash;

#[derive(Default,Clone,PartialEq,PartialOrd,Eq,Ord,Debug,Serialize,Deserialize)]
#[repr(C)]
pub struct TargetCompact(u32);

#[derive(Default,Clone,PartialEq,PartialOrd,Eq,Ord,Debug,Serialize,Deserialize)]
#[repr(C)]
pub struct BlockHeader {
    pub version: u16,
    pub prev_hash: Blake2bHash,
    pub interlink_hash: Blake2bHash,
    pub body_hash: Blake2bHash,
    pub accounts_hash: Blake2bHash,
    pub n_bits: TargetCompact,
    pub height: u32,
    pub timestamp: u32,
    pub nonce: u32
}
