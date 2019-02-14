use beserial::{Deserialize, Serialize};
use crate::{Block, BlockHeader};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChainProof {
    #[beserial(len_type(u16))]
    pub prefix: Vec<Block>,
    #[beserial(len_type(u16))]
    pub suffix: Vec<BlockHeader>,
}
