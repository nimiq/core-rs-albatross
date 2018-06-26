use beserial_derive;
use beserial::{Serialize, SerializeWithLength, Deserialize, DeserializeWithLength};
use super::primitive::hash::Blake2bHash;
use consensus::base::primitive::Address;
use consensus::base::transaction::Transaction;

#[derive(Default,Clone,PartialEq,PartialOrd,Eq,Ord,Debug,Serialize,Deserialize)]
pub struct TargetCompact(u32);

#[derive(Default,Clone,PartialEq,PartialOrd,Eq,Ord,Debug,Serialize,Deserialize)]
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

#[derive(Default,Clone,PartialEq,PartialOrd,Eq,Ord,Debug,Serialize,Deserialize)]
pub struct BlockBody {
    pub miner: Address,
    #[beserial(len_type(u8))]
    pub extra_data: Vec<u8>,
    #[beserial(len_type(u16))]
    pub transactions: Vec<Transaction>,
    //#[beserial(len_type = u16)]
    //pub pruned_accounts: Vec<Account>
}

