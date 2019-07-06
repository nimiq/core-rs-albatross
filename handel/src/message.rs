use std::fmt::Debug;

use beserial::{Serialize, Deserialize};

use crate::multisig::{MultiSignature, IndividualSignature};


#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AggregationMessage<M: Clone + Debug + Serialize + Deserialize> {
    message: M,
    multisig: MultiSignature,
    individual: Option<IndividualSignature>,
    level: u8,
}

impl<M: Clone + Debug + Serialize + Deserialize> AggregationMessage<M> {
    pub fn new(message: M, multisig: MultiSignature, individual: Option<IndividualSignature>, level: usize) -> Self {
        assert!(level <= u8::max_value() as usize);
        Self {
            message,
            multisig,
            individual,
            level: level as u8,
        }
    }
}
