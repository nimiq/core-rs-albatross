use std::{str::FromStr, sync::Mutex};

use nimiq_blockchain::{Blockchain, PostValidationHook};
use nimiq_blockchain_interface::{AbstractBlockchain, PushError, PushResult};
use nimiq_keys::{KeyPair, PrivateKey};
use nimiq_primitives::networks::NetworkId;
use nimiq_test_log::test;
use nimiq_test_utils::{
    block_production::TemporaryBlockProducer,
    blockchain::{generate_transactions, REWARD_KEY},
};

/// Records each call as the hook sees it: the result of an accepted push, `None` for a rejection.
#[derive(Default)]
struct RecordingHook(Mutex<Vec<Option<PushResult>>>);

impl PostValidationHook for RecordingHook {
    fn post_validation(&self, push_result: Result<&PushResult, &PushError>) {
        self.0.lock().unwrap().push(push_result.ok().cloned());
    }
}

/// A publisher hooked into the push must be told exactly once whether its block was accepted.
#[test]
fn it_calls_the_hook_once_with_the_push_outcome() {
    let temp_producer = TemporaryBlockProducer::new();
    let key_pair = KeyPair::from(PrivateKey::from_str(REWARD_KEY).unwrap());
    let height = temp_producer.blockchain.read().block_number();
    let tx = generate_transactions(&key_pair, height, NetworkId::UnitAlbatross, 1, 7)
        .pop()
        .unwrap();
    temp_producer.next_block_with_txs(vec![], false, vec![tx.clone()]);

    // Replaying the transaction passes block production but not the push.
    let duplicate = temp_producer.next_block_no_push_with_txs(vec![], false, vec![tx]);
    let hook = RecordingHook::default();
    let result =
        Blockchain::push_with_hook(temp_producer.blockchain.upgradable_read(), duplicate, &hook);
    assert_eq!(result, Err(PushError::DuplicateTransaction));
    assert_eq!(*hook.0.lock().unwrap(), vec![None]);

    let valid = temp_producer.next_block_no_push(vec![], false);
    let hook = RecordingHook::default();
    let result =
        Blockchain::push_with_hook(temp_producer.blockchain.upgradable_read(), valid, &hook);
    assert_eq!(result, Ok(PushResult::Extended));
    assert_eq!(*hook.0.lock().unwrap(), vec![Some(PushResult::Extended)]);
}
