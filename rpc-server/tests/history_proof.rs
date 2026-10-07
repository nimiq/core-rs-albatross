use std::sync::Arc;

use nimiq_block::Block;
use nimiq_blockchain::{interface::HistoryInterface, BlockProducer, Blockchain, BlockchainConfig};
use nimiq_database::mdbx::MdbxDatabase;
use nimiq_hash::{Blake2bHash, Hash};
use nimiq_mmr::mmr::proof::Proof;
use nimiq_primitives::{networks::NetworkId, policy::Policy};
use nimiq_rpc_interface::{blockchain::BlockchainInterface, types::HistoryProofData};
use nimiq_rpc_server::{dispatchers::BlockchainDispatcher, Error};
use nimiq_serde::Deserialize;
use nimiq_test_log::test;
use nimiq_test_utils::blockchain::{produce_macro_blocks_with_txns, signing_key, voting_key};
use nimiq_transaction::historic_transaction::{HistoricTransaction, HistoricTransactionData};
use nimiq_utils::time::OffsetTime;
use parking_lot::RwLock;

fn blockchain(index_history: bool) -> Arc<RwLock<Blockchain>> {
    Arc::new(RwLock::new(
        Blockchain::new(
            MdbxDatabase::new_volatile(Default::default()).unwrap(),
            BlockchainConfig {
                index_history,
                ..Default::default()
            },
            NetworkId::UnitAlbatross,
            Arc::new(OffsetTime::new()),
        )
        .unwrap(),
    ))
}

/// A chain with transactions in every micro block, through the first election block and the
/// first checkpoint block after it.
fn chain_with_transactions() -> Arc<RwLock<Blockchain>> {
    let blockchain = blockchain(true);
    let producer = BlockProducer::new(signing_key(), voting_key());
    produce_macro_blocks_with_txns(
        &producer,
        &blockchain,
        Policy::batches_per_epoch() as usize + 1,
        2,
        7,
    );
    blockchain
}

fn macro_block(blockchain: &Arc<RwLock<Blockchain>>, block_number: u32) -> Block {
    let block = blockchain
        .read()
        .chain_store
        .get_block_at(block_number, false, None)
        .unwrap();
    assert!(block.is_macro());
    block
}

/// The hash of a basic transaction in the given block.
fn transaction_in(blockchain: &Arc<RwLock<Blockchain>>, block_number: u32) -> Blake2bHash {
    blockchain
        .read()
        .history_store
        .get_block_transactions(block_number, None)
        .into_iter()
        .find(|tx| matches!(tx.data, HistoricTransactionData::Basic(_)))
        .expect("block has a transaction")
        .tx_hash()
        .into()
}

/// Checks a served proof the way an independent verifier would: from the served bytes and the
/// block's `history_root` only.
fn assert_proof_verifies(proof: &HistoryProofData, tx_hash: &Blake2bHash, block: &Block) {
    let hist_tx = HistoricTransaction::deserialize_from_vec(
        &hex::decode(&proof.historic_transaction).unwrap(),
    )
    .unwrap();
    assert_eq!(&Blake2bHash::from(hist_tx.tx_hash()), tx_hash);

    let mmr_proof = Proof {
        mmr_size: proof.mmr_size as usize,
        nodes: proof.nodes.clone(),
    };
    assert_eq!(
        mmr_proof.verify(
            block.history_root(),
            &[(proof.leaf_index as usize, &hist_tx)]
        ),
        Ok(true)
    );
}

#[test(tokio::test)]
async fn history_proof_verifies_against_a_checkpoint_and_the_election_block() {
    let blockchain = chain_with_transactions();
    let dispatcher = BlockchainDispatcher::new(blockchain.clone().into());

    // A transaction in the last micro block before the checkpoint.
    let checkpoint = Policy::genesis_block_number() + Policy::blocks_per_batch();
    let tx_hash = transaction_in(&blockchain, checkpoint - 1);
    let election = Policy::election_block_after(checkpoint);

    for macro_number in [checkpoint, election] {
        let proof = dispatcher
            .get_transaction_history_proof(tx_hash.clone(), macro_number)
            .await
            .unwrap()
            .data;
        assert_proof_verifies(&proof, &tx_hash, &macro_block(&blockchain, macro_number));
    }
}

#[test(tokio::test)]
async fn history_proof_rejects_a_macro_block_the_transaction_is_not_in() {
    let blockchain = chain_with_transactions();
    let dispatcher = BlockchainDispatcher::new(blockchain.clone().into());

    // Before the transaction.
    let first_checkpoint = Policy::genesis_block_number() + Policy::blocks_per_batch();
    let tx_hash = transaction_in(&blockchain, first_checkpoint + 1);
    assert!(matches!(
        dispatcher
            .get_transaction_history_proof(tx_hash.clone(), first_checkpoint)
            .await,
        Err(Error::TransactionNotInHistoryAt(hash, number))
            if hash == tx_hash && number == first_checkpoint
    ));

    // In the next epoch.
    let next_epoch_checkpoint =
        Policy::election_block_after(first_checkpoint) + Policy::blocks_per_batch();
    assert!(matches!(
        dispatcher
            .get_transaction_history_proof(tx_hash, next_epoch_checkpoint)
            .await,
        Err(Error::TransactionNotInHistoryAt(..))
    ));
}

#[test(tokio::test)]
async fn history_proof_rejects_bad_requests() {
    let blockchain = chain_with_transactions();
    let dispatcher = BlockchainDispatcher::new(blockchain.clone().into());

    let first_block = Policy::genesis_block_number() + 1;
    let tx_hash = transaction_in(&blockchain, first_block);
    let checkpoint = Policy::macro_block_after(first_block);

    assert!(matches!(
        dispatcher
            .get_transaction_history_proof(tx_hash.clone(), checkpoint + 1)
            .await,
        Err(Error::NotAMacroBlock(number)) if number == checkpoint + 1
    ));
    assert!(matches!(
        dispatcher
            .get_transaction_history_proof(Blake2bHash::default(), checkpoint)
            .await,
        Err(Error::TransactionNotFound(_))
    ));

    // A macro block number past the head.
    let future = Policy::election_block_after(first_block) + 2 * Policy::blocks_per_batch();
    assert!(matches!(
        dispatcher.get_transaction_history_proof(tx_hash, future).await,
        Err(Error::BlockNotFound(number)) if number == future
    ));
}

#[test(tokio::test)]
async fn history_proof_requires_the_history_index() {
    let dispatcher = BlockchainDispatcher::new(blockchain(false).into());
    assert!(matches!(
        dispatcher
            .get_transaction_history_proof(Blake2bHash::default(), Policy::genesis_block_number())
            .await,
        Err(Error::RequiresHistoryIndex)
    ));
}

#[test(tokio::test)]
async fn raw_macro_header_hashes_to_the_block_hash() {
    let blockchain = chain_with_transactions();
    let dispatcher = BlockchainDispatcher::new(blockchain.clone().into());

    let checkpoint = Policy::genesis_block_number() + Policy::blocks_per_batch();
    for macro_number in [checkpoint, Policy::election_block_after(checkpoint)] {
        let content = hex::decode(
            dispatcher
                .get_raw_macro_header(macro_number)
                .await
                .unwrap()
                .data,
        )
        .unwrap();
        assert_eq!(
            content.hash::<Blake2bHash>(),
            macro_block(&blockchain, macro_number).hash()
        );
    }
}

#[test(tokio::test)]
async fn raw_macro_header_rejects_micro_and_missing_blocks() {
    let blockchain = chain_with_transactions();
    let dispatcher = BlockchainDispatcher::new(blockchain.into());

    let micro = Policy::genesis_block_number() + 1;
    assert!(matches!(
        dispatcher.get_raw_macro_header(micro).await,
        Err(Error::NotAMacroBlock(number)) if number == micro
    ));

    let future = Policy::election_block_after(micro) + 2 * Policy::blocks_per_batch();
    assert!(matches!(
        dispatcher.get_raw_macro_header(future).await,
        Err(Error::BlockNotFound(number)) if number == future
    ));
}
