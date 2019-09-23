// TODO I need help cleaning this up
// How do I get the reference to a validator to here?

use std::sync::Arc;

use bls::bls12_381::{CompressedPublicKey, CompressedSignature};
use json::JsonValue;

use crate::handler::Method;
use crate::handlers::Module;

pub struct BlockProductionAlbatrossHandler {
    key: CompressedPublicKey,
    pok: CompressedSignature,
}

impl BlockProductionAlbatrossHandler {
    pub fn new(key: CompressedPublicKey, pok: CompressedSignature) -> Self {
        Self { key, pok }
    }

    fn validator_key(&self, _params: &[JsonValue]) -> Result<JsonValue, JsonValue> {
        Ok(object! {
            "validatorKey" => hex::encode(&self.key),
            "proofOfKnowledge" => hex::encode(&self.pok),
        })
    }
}

impl Module for BlockProductionAlbatrossHandler {
    rpc_module_methods! {
        "validatorKey" => validator_key,
    }
}
