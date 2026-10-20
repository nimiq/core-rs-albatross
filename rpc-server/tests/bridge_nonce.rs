//! `getBridgeNonce` on a node whose accounts tree is complete, and on one still syncing it.

use nimiq_keys::Address;
use nimiq_primitives::key_nibbles::KeyNibbles;
use nimiq_rpc_interface::blockchain::BlockchainInterface;
use nimiq_rpc_server::{dispatchers::BlockchainDispatcher, Error};
use nimiq_test_log::test;
use nimiq_test_utils::bridge::{key, record_leaf, BridgeChain, TARGET_KEY};

const AMOUNT: u64 = 500;

/// A node that has synced the bridge's account but not its nonce store cannot tell a target's
/// nonce, so it reports that it has no consensus instead of failing the request.
#[test(tokio::test)]
async fn get_bridge_nonce_has_no_consensus_until_the_nonce_store_is_synced() {
    let chain = BridgeChain::new(10);
    let target = key(TARGET_KEY);
    let address = Address::from(&target.public);
    let burn = chain.burn(&address, AMOUNT, 1);
    chain.push_block(vec![chain.update(vec![record_leaf(&burn)])]);
    chain.push_block(vec![chain.release(&target, AMOUNT, burn, 0, &[])]);

    let complete = BlockchainDispatcher::new(chain.node.blockchain.clone().into());
    let nonce = complete
        .get_bridge_nonce(chain.bridge.clone(), address.clone())
        .await
        .unwrap()
        .data;
    assert_eq!(nonce, 1);

    // The target's nonce is the only entry in the bridge's store, so a chunk ending inside the store
    // ends at it.
    let bridge = KeyNibbles::from(&chain.bridge);
    let node = chain.syncing_node(|end| bridge < *end && bridge.is_prefix_of(end));
    let syncing = BlockchainDispatcher::new(node.blockchain.clone().into());
    assert!(matches!(
        syncing
            .get_bridge_nonce(chain.bridge.clone(), address)
            .await,
        Err(Error::NoConsensus)
    ));
}
