use std::sync::Arc;

use nimiq_blockchain::{BlockProducer, Blockchain, BlockchainConfig};
use nimiq_blockchain_interface::AbstractBlockchain;
use nimiq_blockchain_proxy::BlockchainProxy;
use nimiq_consensus::messages::{MacroChainError, RequestMacroChain};
use nimiq_database::mdbx::MdbxDatabase;
use nimiq_network_interface::request::Handle;
use nimiq_network_mock::{MockNetwork, MockPeerId};
use nimiq_primitives::{networks::NetworkId, policy::Policy};
use nimiq_test_log::test;
use nimiq_test_utils::blockchain::{produce_macro_blocks, signing_key, voting_key};
use nimiq_utils::time::OffsetTime;
use parking_lot::RwLock;

/// Helper to create a test blockchain with some blocks
fn create_test_blockchain(num_macro_blocks: usize) -> Arc<RwLock<Blockchain>> {
    let blockchain = Arc::new(RwLock::new(
        Blockchain::new(
            MdbxDatabase::new_volatile(Default::default()).unwrap(),
            BlockchainConfig::default(),
            NetworkId::UnitAlbatross,
            Arc::new(OffsetTime::new()),
        )
        .unwrap(),
    ));

    let producer = BlockProducer::new(signing_key(), voting_key());
    produce_macro_blocks(&producer, &blockchain, num_macro_blocks);

    blockchain
}

#[test(tokio::test)]
async fn test_request_macro_chain_rejects_micro_blocks() {
    // Create a blockchain with at least one batch (so we have micro blocks)
    let blockchain = create_test_blockchain(2);
    let blockchain_proxy = BlockchainProxy::from(&blockchain);

    // Find a micro block hash on the main chain
    let micro_block_hash = {
        let blockchain = blockchain.read();
        let current_block = blockchain.block_number();

        let mut found_hash = None;
        for block_num in 1..=current_block {
            if !Policy::is_macro_block_at(block_num) {
                if let Ok(block) = blockchain.get_block_at(block_num, false, None) {
                    if block.is_micro() {
                        found_hash = Some(block.hash());
                        break;
                    }
                }
            }
        }
        found_hash.expect("Should have at least one micro block")
    };

    // Create a request with only the micro block hash as locator
    let request = RequestMacroChain {
        locators: vec![micro_block_hash],
        max_epochs: 10,
    };

    // Handle the request
    let peer_id = MockPeerId(1);
    let result = <RequestMacroChain as Handle<MockNetwork, BlockchainProxy>>::handle(
        &request,
        peer_id,
        &blockchain_proxy,
    );

    // Should return UnknownLocators error instead of panicking
    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        MacroChainError::UnknownLocators
    ));
}

#[test(tokio::test)]
async fn test_request_macro_chain_accepts_macro_blocks() {
    // Create a blockchain with multiple macro blocks
    let blockchain = create_test_blockchain(3);
    let blockchain_proxy = BlockchainProxy::from(&blockchain);

    // Get the first macro block hash from the chain (not the last one)
    let macro_block_hash = {
        let blockchain = blockchain.read();

        // Find the first macro block (genesis or first checkpoint)
        let mut found_hash = None;
        for block_num in 1..=blockchain.block_number() {
            if Policy::is_macro_block_at(block_num) {
                if let Ok(block) = blockchain.get_block_at(block_num, false, None) {
                    if block.is_macro() {
                        found_hash = Some(block.hash());
                        break; // Use the first one, not the last
                    }
                }
            }
        }
        found_hash.expect("Should have at least one macro block")
    };

    // Create a request with the macro block hash as locator
    let request = RequestMacroChain {
        locators: vec![macro_block_hash],
        max_epochs: 10,
    };

    // Handle the request
    let peer_id = MockPeerId(1);
    let result = <RequestMacroChain as Handle<MockNetwork, BlockchainProxy>>::handle(
        &request,
        peer_id,
        &blockchain_proxy,
    );

    // Should succeed
    assert!(result.is_ok());
    let response = result.unwrap();

    // Should return macro blocks after the locator (or be empty if locator is the last block)
    // Since we used the first macro block, there should be more blocks after it
    assert!(!response.epochs.is_empty() || response.checkpoint.is_some());
}

#[test(tokio::test)]
async fn test_request_macro_chain_skips_micro_blocks() {
    // Create a blockchain with multiple batches
    let blockchain = create_test_blockchain(3);
    let blockchain_proxy = BlockchainProxy::from(&blockchain);

    // Find both a micro block and a macro block (not the last macro block)
    let (micro_block_hash, macro_block_hash) = {
        let blockchain = blockchain.read();
        let current_block = blockchain.block_number();

        let mut micro_hash = None;
        let mut macro_hash = None;

        // Find first micro and first macro block
        for block_num in 1..=current_block {
            if let Ok(block) = blockchain.get_block_at(block_num, false, None) {
                if block.is_micro() && micro_hash.is_none() {
                    micro_hash = Some(block.hash());
                } else if block.is_macro() && macro_hash.is_none() {
                    macro_hash = Some(block.hash());
                }

                if micro_hash.is_some() && macro_hash.is_some() {
                    break;
                }
            }
        }

        (
            micro_hash.expect("Should have at least one micro block"),
            macro_hash.expect("Should have at least one macro block"),
        )
    };

    // Create a request with micro block first, then macro block
    // The handler should skip the micro block and use the macro block
    let request = RequestMacroChain {
        locators: vec![micro_block_hash, macro_block_hash],
        max_epochs: 10,
    };

    // Handle the request
    let peer_id = MockPeerId(1);
    let result = <RequestMacroChain as Handle<MockNetwork, BlockchainProxy>>::handle(
        &request,
        peer_id,
        &blockchain_proxy,
    );

    // Should succeed by using the macro block
    assert!(result.is_ok());
    let response = result.unwrap();

    // Should return macro blocks starting from the macro block locator (or checkpoint)
    assert!(!response.epochs.is_empty() || response.checkpoint.is_some());
}

#[test(tokio::test)]
async fn test_request_macro_chain_too_many_locators() {
    let blockchain = create_test_blockchain(2);
    let blockchain_proxy = BlockchainProxy::from(&blockchain);

    // Create a request with too many locators (> MAX_LOCATORS = 100)
    let locators = vec![Default::default(); 101];
    let request = RequestMacroChain {
        locators,
        max_epochs: 10,
    };

    // Handle the request
    let peer_id = MockPeerId(1);
    let result = <RequestMacroChain as Handle<MockNetwork, BlockchainProxy>>::handle(
        &request,
        peer_id,
        &blockchain_proxy,
    );

    // Should return TooManyLocators error
    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        MacroChainError::TooManyLocators
    ));
}
