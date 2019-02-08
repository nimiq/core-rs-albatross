use beserial::{Deserialize, Serialize};
#[cfg(feature = "block")]
use crate::block::{Block, BlockHeader};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChainProof {
    #[beserial(len_type(u16))]
    pub prefix: Vec<Block>,
    #[beserial(len_type(u16))]
    pub suffix: Vec<BlockHeader>
}
