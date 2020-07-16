use crate::{Block, BlockHeader};
use beserial::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChainProof {
    #[beserial(len_type(u16))]
    pub prefix: Vec<Block>,
    #[beserial(len_type(u16))]
    pub suffix: Vec<BlockHeader>,
}
