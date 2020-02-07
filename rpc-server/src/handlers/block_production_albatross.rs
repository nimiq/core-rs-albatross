// TODO I need help cleaning this up
// How do I get the reference to a validator to here?

use std::string::ToString;
use std::sync::Arc;

use json::{object, JsonValue};

use validator::validator::Validator;

use crate::handler::Method;
use crate::handlers::Module;

pub struct BlockProductionAlbatrossHandler {
    validator: Arc<Validator>,
}

impl BlockProductionAlbatrossHandler {
    pub fn new(validator: Arc<Validator>) -> Self {
        Self { validator }
    }

    fn validator_key(&self, _params: &[JsonValue]) -> Result<JsonValue, JsonValue> {
        let key_pair = &self.validator.validator_key;

        // Compute proof of knowledge.
        // TODO: Do we need this at all? This is only needed to sign staking transactions, and
        // that can be done with the mempool module.
        let proof_of_knowledge = key_pair.sign(&key_pair.public_key).compress();

        Ok(object! {
            "validatorKey" => self.validator.validator_key.public_key.to_string(),
            "proofOfKnowledge" => proof_of_knowledge.to_string(),
        })
    }

    fn proof_of_knowledge(&self, _params: &[JsonValue]) -> Result<JsonValue, JsonValue> {
        let key_pair = &self.validator.validator_key;

        // Compute proof of knowledge.
        // TODO: Do we need this at all? This is only needed to sign staking transactions, and
        // that can be done with the mempool module.
        let proof_of_knowledge = key_pair.sign(&key_pair.public_key).compress();

        Ok(object! {
            "proofOfKnowledge" => proof_of_knowledge.to_string(),
        })
    }
}

impl Module for BlockProductionAlbatrossHandler {
    rpc_module_methods! {
        "validatorKey" => validator_key,
        "proofOfKnowledge" => proof_of_knowledge,
    }
}
