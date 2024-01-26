use nimiq_block::{
    Block, DoubleProposalProof, DoubleVoteProof, EquivocationProof, ForkProof, MacroHeader,
    MicroHeader,
};
use nimiq_blockchain_interface::{AbstractBlockchain, PushResult};
use nimiq_bls::AggregateSignature;
use nimiq_database::traits::WriteTransaction;
use nimiq_genesis::NetworkId;
use nimiq_hash::{Blake2bHash, Blake2sHash, Hash, HashOutput};
use nimiq_keys::{KeyPair, PrivateKey};
use nimiq_primitives::{policy::Policy, TendermintIdentifier, TendermintStep, TendermintVote};
use nimiq_serde::Deserialize;
use nimiq_test_log::test;
use nimiq_test_utils::{
    block_production::TemporaryBlockProducer,
    blockchain::{generate_transactions, produce_macro_blocks, validator_address},
    test_custom_block::{next_micro_block, BlockConfig},
};
use nimiq_transaction::{
    historic_transaction::{
        EquivocationEvent, HistoricTransaction, HistoricTransactionData, JailEvent, PenalizeEvent,
        RewardEvent,
    },
    ExecutedTransaction, Transaction,
};

fn key_pair_with_funds() -> KeyPair {
    let priv_key: PrivateKey =
        Deserialize::deserialize_from_vec(
            &hex::decode("6c9320ac201caf1f8eaa5b05f5d67a9e77826f3f6be266a0ecccc20416dc6587")
                .unwrap()[..],
        )
        .unwrap();
    priv_key.into()
}

fn get_hist_tx(temp_producer: &TemporaryBlockProducer) -> Vec<HistoricTransaction> {
    let blockchain = temp_producer.blockchain.read();
    blockchain.history_store.get_epoch_transactions(1, None)
}

// Revert block and assert that history store is reverted as well.
fn revert_block(temp_producer: &TemporaryBlockProducer, hist_tx_pre: &[HistoricTransaction]) {
    let blockchain = temp_producer.blockchain.read();
    let mut txn = blockchain.write_transaction();
    blockchain.revert_blocks(1, &mut txn).unwrap();
    txn.commit();

    let hist_tx_revert = get_hist_tx(&temp_producer);

    assert_eq!(hist_tx_pre, &hist_tx_revert);
}

fn setup_blockchain_with_history() -> (TemporaryBlockProducer, TemporaryBlockProducer) {
    let temp_producer1 = TemporaryBlockProducer::new();
    let temp_producer2 = TemporaryBlockProducer::new();
    assert_eq!(
        temp_producer2.push(temp_producer1.next_block(vec![], false)),
        Ok(PushResult::Extended)
    );

    add_block_assert_history_store(&temp_producer1, vec![], vec![], true);

    let block = temp_producer1.blockchain.read().head();

    assert_eq!(&temp_producer2.push(block), &Ok(PushResult::Extended));

    (temp_producer1, temp_producer2)
}

fn do_fork(
    temp_producer1: &TemporaryBlockProducer,
    temp_producer2: &TemporaryBlockProducer,
) -> EquivocationProof {
    let config = BlockConfig::default();

    // Produce a fork in producer 1.

    // Generates the first block_1a.
    let block_1a = temp_producer1.next_block(vec![], false).unwrap_micro();
    let header_1a = block_1a.header.clone();

    // Generates the fork block block_2a.
    let block_2a = {
        let blockchain = &temp_producer2.blockchain.read();
        next_micro_block(&temp_producer1.producer.signing_key, blockchain, &config)
    };
    let header_2a = block_2a.header.clone();

    // Pushes fork block to producer 1.
    assert_eq!(
        &temp_producer1.push(Block::Micro(block_2a.clone())),
        &Ok(PushResult::Forked)
    );
    // Also adds the block to producer 2 (but this one will not have a fork).
    assert_eq!(
        &temp_producer2.push(Block::Micro(block_1a)),
        &Ok(PushResult::Extended)
    );

    // Builds the equivocation proof.
    let signing_key = temp_producer1.producer.signing_key.clone();
    let justification1 = signing_key.sign(MicroHeader::hash::<Blake2bHash>(&header_1a).as_bytes());
    let justification2 = signing_key.sign(MicroHeader::hash::<Blake2bHash>(&header_2a).as_bytes());

    ForkProof::new(
        validator_address(),
        header_1a,
        justification1,
        header_2a.clone(),
        justification2,
    )
    .into()
}

fn do_double_proposal(
    temp_producer1: &TemporaryBlockProducer,
    temp_producer2: &TemporaryBlockProducer,
) -> EquivocationProof {
    // Make double proposal on macro block.
    produce_macro_blocks(&temp_producer1.producer, &temp_producer1.blockchain, 1);
    produce_macro_blocks(&temp_producer2.producer, &temp_producer2.blockchain, 1);
    let signing_key = temp_producer1.producer.signing_key.clone();

    let header1 = temp_producer1
        .blockchain
        .read()
        .head()
        .unwrap_macro()
        .header;
    let header2 = temp_producer2
        .blockchain
        .read()
        .head()
        .unwrap_macro()
        .header;
    let justification1 = signing_key.sign(MacroHeader::hash::<Blake2bHash>(&header1).as_bytes());
    let justification2 = signing_key.sign(MacroHeader::hash::<Blake2bHash>(&header2).as_bytes());

    // Produce the double proposal proof.
    EquivocationProof::DoubleProposal(DoubleProposalProof::new(
        validator_address(),
        header1.clone(),
        justification1,
        header2,
        justification2,
    ))
}

fn do_double_vote(temp_producer1: &TemporaryBlockProducer) -> EquivocationProof {
    // Make double proposal on macro block.
    produce_macro_blocks(&temp_producer1.producer, &temp_producer1.blockchain, 1);

    let voting_key = temp_producer1.producer.voting_key.clone();
    let header = temp_producer1
        .blockchain
        .read()
        .head()
        .unwrap_macro()
        .header;

    let validators = temp_producer1
        .blockchain
        .read()
        .get_validators_for_epoch(Policy::epoch_at(header.block_number), None)
        .unwrap();
    let validator = validators.validators[0].clone();
    let slots = validator.slots;

    let tendermint_id = TendermintIdentifier {
        block_number: header.block_number,
        round_number: header.round,
        step: TendermintStep::PreVote,
    };
    let signature1 = voting_key.sign(&TendermintVote {
        proposal_hash: None,
        id: tendermint_id.clone(),
    });
    let signature2 = voting_key.sign(&TendermintVote {
        proposal_hash: Some(Blake2sHash::default()),
        id: tendermint_id.clone(),
    });

    // Produce the double proposal proof.
    EquivocationProof::DoubleVote(DoubleVoteProof::new(
        tendermint_id,
        validator_address(),
        None,
        AggregateSignature::from_signatures(&slots.clone().map(|_| signature1).collect::<Vec<_>>()),
        slots.clone().map(|i| i.into()).collect(),
        Some(Blake2sHash::default()),
        AggregateSignature::from_signatures(&slots.clone().map(|_| signature2).collect::<Vec<_>>()),
        slots.clone().map(|i| i.into()).collect(),
    ))
}

fn add_block_assert_history_store(
    temp_producer1: &TemporaryBlockProducer,
    equivocation_proofs: Vec<EquivocationProof>,
    transactions: Vec<Transaction>,
    is_skip_block: bool,
) -> (Vec<HistoricTransaction>, Vec<HistoricTransaction>) {
    // Get initial history store.
    let hist_tx_pre = get_hist_tx(&temp_producer1);

    // Add a block with the equivocation proofs.
    let skip_block_proof = if is_skip_block {
        Some(temp_producer1.create_skip_block_proof())
    } else {
        None
    };
    let micro_block = next_micro_block(
        &temp_producer1.producer.signing_key,
        &temp_producer1.blockchain.read(),
        &BlockConfig {
            equivocation_proofs: equivocation_proofs.clone(),
            test_macro: false,
            test_election: false,
            skip_block_proof,
            transactions: transactions.clone(),
            ..Default::default()
        },
    );
    let header = micro_block.header.clone();
    let is_skip_block = micro_block.is_skip_block();

    assert_eq!(
        &temp_producer1.push(Block::Micro(micro_block)),
        &Ok(PushResult::Extended)
    );

    // Get the new history store after the block push.
    let hist_tx_after = get_hist_tx(&temp_producer1);

    // Assert that the jail inherent and equivocation proofs are present in history store.
    // There is 1 inherent and 1 event generated per equivocation proof.
    let mut expected_len = hist_tx_pre.len() + equivocation_proofs.len() * 2 + transactions.len();
    if is_skip_block {
        expected_len += 1;
    }
    assert_eq!(hist_tx_after.len(), expected_len);

    let mut i = hist_tx_pre.len();
    for transaction in transactions {
        let hist_tx_equiv = &hist_tx_after[i];
        i += 1;

        assert_eq!(hist_tx_equiv.block_number, header.block_number);
        assert_eq!(
            hist_tx_equiv.data,
            HistoricTransactionData::Basic(ExecutedTransaction::Ok(transaction))
        );
    }

    for equivocation_proof in equivocation_proofs.iter() {
        let hist_tx_equiv = &hist_tx_after[i];
        i += 1;

        assert_eq!(hist_tx_equiv.block_number, header.block_number);
        assert_eq!(
            hist_tx_equiv.data,
            HistoricTransactionData::Equivocation(EquivocationEvent {
                locator: equivocation_proof.locator()
            })
        );
    }

    for equivocation_proof in equivocation_proofs.iter() {
        let hist_tx_inherent = &hist_tx_after[i];
        i += 1;

        assert_eq!(hist_tx_inherent.block_number, header.block_number);
        assert_eq!(
            hist_tx_inherent.data,
            HistoricTransactionData::Jail(JailEvent {
                validator_address: validator_address(),
                slots: 0..512,
                offense_event_block: equivocation_proof.block_number(),
                new_epoch_slot_range: None
            })
        )
    }

    if is_skip_block {
        let hist_tx_inherent = &hist_tx_after[i];

        assert_eq!(hist_tx_inherent.block_number, header.block_number);
        assert_eq!(
            hist_tx_inherent.data,
            HistoricTransactionData::Penalize(PenalizeEvent {
                validator_address: validator_address(),
                slot: 0,
                offense_event_block: header.block_number
            })
        );
    }

    (hist_tx_pre, hist_tx_after)
}

#[test]
fn it_pushes_and_reverts_fork_equivocation_block() {
    // Adds the same initial block to both producers.
    let (temp_producer1, temp_producer2) = setup_blockchain_with_history();

    let fork_equivocation = do_fork(&temp_producer1, &temp_producer2);

    // Get initial history store.
    let (hist_tx_pre, _) =
        add_block_assert_history_store(&temp_producer1, vec![fork_equivocation], vec![], false);

    // Revert block and assert that history store is reverted as well.
    revert_block(&temp_producer1, &hist_tx_pre);
}

#[test]
fn it_pushes_and_reverts_double_proposal_equivocation_block() {
    // Adds the same initial block to both producers.
    let (temp_producer1, temp_producer2) = setup_blockchain_with_history();

    let double_proposal_proof = do_double_proposal(&temp_producer1, &temp_producer2);

    // Get initial history store.
    let (hist_tx_pre, _) =
        add_block_assert_history_store(&temp_producer1, vec![double_proposal_proof], vec![], false);

    // Revert block and assert that history store is reverted as well.
    revert_block(&temp_producer1, &hist_tx_pre);
}

#[test]
fn it_pushes_and_reverts_double_vote_equivocation_block() {
    // Adds the same initial block to both producers.
    let (temp_producer1, _temp_producer2) = setup_blockchain_with_history();

    let double_vote_proof = do_double_vote(&temp_producer1);

    // Get initial history store.
    let (hist_tx_pre, _) =
        add_block_assert_history_store(&temp_producer1, vec![double_vote_proof], vec![], false);

    // Revert block and assert that history store is reverted as well.
    revert_block(&temp_producer1, &hist_tx_pre);
}

#[test]
fn it_pushes_and_reverts_skip_block() {
    // Adds the same initial block to both producers.
    let (temp_producer1, _temp_producer2) = setup_blockchain_with_history();

    // Get initial history store.
    let hist_tx_pre = get_hist_tx(&temp_producer1);

    // Adds skip block.
    add_block_assert_history_store(&temp_producer1, vec![], vec![], true);

    // Revert block and assert that history store is reverted as well.
    revert_block(&temp_producer1, &hist_tx_pre);
}

#[test]
fn it_pushes_and_reverts_block_with_txns() {
    // Adds the same initial block to both producers.
    let (temp_producer1, _temp_producer2) = setup_blockchain_with_history();

    // Get initial history store.
    let hist_tx_pre = get_hist_tx(&temp_producer1);

    // Adds block with transactions.
    let key_pair = key_pair_with_funds();
    let mut txns = generate_transactions(
        &key_pair,
        temp_producer1.blockchain.read().block_number(),
        NetworkId::UnitAlbatross,
        3,
        0,
    );
    txns.sort_unstable();

    add_block_assert_history_store(&temp_producer1, vec![], txns, false);

    // Revert block and assert that history store is reverted as well.
    revert_block(&temp_producer1, &hist_tx_pre);
}

#[test]
fn it_pushes_macro_block_with_rewards() {
    // Adds the same initial block to both producers.
    let (temp_producer1, _temp_producer2) = setup_blockchain_with_history();

    // Get initial history store.
    let hist_tx_pre = get_hist_tx(&temp_producer1);

    // Adds block with transactions.
    produce_macro_blocks(&temp_producer1.producer, &temp_producer1.blockchain, 2);

    let hist_tx_after = get_hist_tx(&temp_producer1);

    // Simple case. 1x Reward to validator.
    let reward_txs = {
        let macro_block = temp_producer1.blockchain.read().head().unwrap_macro();
        macro_block.body.unwrap().transactions
    };
    assert_eq!(reward_txs.len(), 1);

    assert_eq!(hist_tx_after.len(), hist_tx_pre.len() + reward_txs.len());

    let mut i = hist_tx_pre.len();
    for reward_tx in reward_txs {
        assert_eq!(
            &RewardEvent {
                validator_address: reward_tx.validator_address,
                reward_address: reward_tx.recipient,
                value: reward_tx.value
            },
            hist_tx_after[i].unwrap_reward()
        );
        i += 1;
    }
}
