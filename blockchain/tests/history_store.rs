use nimiq_block::{
    Block, DoubleProposalProof, DoubleVoteProof, EquivocationProof, ForkProof, MicroHeader,
};
use nimiq_blockchain::interface::HistoryInterface;
use nimiq_blockchain_interface::{AbstractBlockchain, PushResult};
use nimiq_bls::{AggregateSignature, KeyPair as BlsKeyPair};
use nimiq_database::{mdbx::MAX_DUP_VALUE_SIZE, traits::WriteTransaction};
use nimiq_genesis::NetworkId;
use nimiq_hash::{Blake2sHash, HashOutput};
use nimiq_keys::{Address, KeyPair, PrivateKey, PublicKey, SecureGenerate, Signature};
use nimiq_primitives::{
    account::AccountType, coin::Coin, policy::Policy, TendermintIdentifier, TendermintProposal,
    TendermintStep, TendermintVote,
};
use nimiq_serde::{Deserialize, Serialize};
use nimiq_test_log::test;
use nimiq_test_utils::{
    block_production::TemporaryBlockProducer,
    blockchain::{generate_transactions, produce_macro_blocks, validator_address},
    test_custom_block::{next_micro_block, BlockConfig},
    test_rng,
};
use nimiq_transaction::{
    account::staking_contract::IncomingStakingTransactionData,
    historic_transaction::{
        EquivocationEvent, HistoricTransaction, HistoricTransactionData, JailEvent, PenalizeEvent,
        RewardEvent,
    },
    ExecutedTransaction, SignatureProof, Transaction,
};
use nimiq_transaction_builder::TransactionBuilder;
use nimiq_utils::merkle::Blake2bMerklePath;

fn key_pair_with_funds() -> KeyPair {
    let priv_key = PrivateKey::deserialize_from_vec(
        &hex::decode("6c9320ac201caf1f8eaa5b05f5d67a9e77826f3f6be266a0ecccc20416dc6587").unwrap(),
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

    let hist_tx_revert = get_hist_tx(temp_producer);

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

    let block = temp_producer1.blockchain.read().head().clone();

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
    let justification1 = signing_key.sign(MicroHeader::hash(&header_1a).as_bytes());
    let justification2 = signing_key.sign(MicroHeader::hash(&header_2a).as_bytes());

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
        .clone()
        .unwrap_macro()
        .header;
    let header2 = temp_producer2
        .blockchain
        .read()
        .head()
        .clone()
        .unwrap_macro()
        .header;
    let proposal1 = TendermintProposal {
        proposal: header1,
        round: 0,
        valid_round: None,
    };
    let proposal2 = TendermintProposal {
        proposal: header2,
        round: 0,
        valid_round: None,
    };
    let justification1 = signing_key.sign(proposal1.hash().as_bytes());
    let justification2 = signing_key.sign(proposal2.hash().as_bytes());

    // Produce the double proposal proof.
    EquivocationProof::DoubleProposal(DoubleProposalProof::new(
        validator_address(),
        proposal1,
        justification1,
        proposal2,
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
        .clone()
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
        network: header.network,
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
    let hist_tx_pre = get_hist_tx(temp_producer1);

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
    let hist_tx_after = get_hist_tx(temp_producer1);

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
        let macro_block = temp_producer1
            .blockchain
            .read()
            .head()
            .clone()
            .unwrap_macro();
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

/// Returns a merkle path of 31 nodes, the most a `SignatureProof` accepts. The node hashes are
/// arbitrary, since the merkle path only determines the address that the proof signs for.
fn max_size_merkle_path(seed: u8) -> Blake2bMerklePath {
    let num_nodes: u8 = 31;
    // Node count, the left/right bits as a vector of 4 bytes, then the vector of node hashes
    let mut bytes = vec![num_nodes, 4, 0, 0, 0, 0, num_nodes];
    for i in 0..num_nodes {
        bytes.extend_from_slice(&[seed.wrapping_add(i); 32]);
    }
    let merkle_path = Blake2bMerklePath::deserialize_from_vec(&bytes).unwrap();
    assert!(merkle_path.serialized_size() <= Policy::MAX_MERKLE_PATH_SIZE);
    merkle_path
}

/// Creates a valid validator creation whose sender and validator address are both signed for
/// through maximal merkle paths, which makes the transaction too large for a dup table value.
/// Returns the transaction and its sender, which still needs to be funded.
fn create_oversized_validator(validity_start_height: u32) -> (Transaction, Address) {
    let mut rng = test_rng(true);
    let sender_key_pair = KeyPair::generate(&mut rng);
    let cold_key_pair = KeyPair::generate(&mut rng);
    let signing_key_pair = KeyPair::generate(&mut rng);
    let voting_key_pair = BlsKeyPair::generate(&mut rng);

    let sender_merkle_path = max_size_merkle_path(0);
    let sender = Address::from(sender_merkle_path.compute_root(&sender_key_pair.public));

    let mut data = IncomingStakingTransactionData::CreateValidator {
        signing_key: signing_key_pair.public,
        voting_key: voting_key_pair.public_key.compress(),
        reward_address: Address::from(&sender_key_pair),
        signal_data: None,
        proof_of_knowledge: voting_key_pair.sign(&voting_key_pair.public_key).compress(),
        proof: SignatureProof::default(),
    };
    let mut tx = Transaction::new_extended(
        sender.clone(),
        AccountType::Basic,
        vec![],
        Policy::STAKING_CONTRACT_ADDRESS,
        AccountType::Staking,
        data.serialize_to_vec(),
        Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT),
        Coin::from_u64_unchecked(100),
        validity_start_height,
        NetworkId::UnitAlbatross,
    );

    // The validator proof signs the transaction with an empty proof in the recipient data
    data.set_signature(SignatureProof {
        public_key: PublicKey::Ed25519(cold_key_pair.public),
        merkle_path: max_size_merkle_path(100),
        signature: Signature::Ed25519(cold_key_pair.sign(&tx.serialize_content())),
        webauthn_fields: None,
    });
    tx.recipient_data = data.serialize_to_vec();

    tx.proof = SignatureProof {
        public_key: PublicKey::Ed25519(sender_key_pair.public),
        merkle_path: sender_merkle_path,
        signature: Signature::Ed25519(sender_key_pair.sign(&tx.serialize_content())),
        webauthn_fields: None,
    }
    .serialize_to_vec();

    tx.verify(NetworkId::UnitAlbatross, Policy::max_supported_version())
        .expect("Transaction must be valid");
    (tx, sender)
}

#[test]
fn it_stores_and_reverts_oversized_transactions() {
    let temp_producer1 = TemporaryBlockProducer::new();
    let temp_producer2 = TemporaryBlockProducer::new();

    let validity_start_height = temp_producer1.blockchain.read().block_number() + 1;
    let (create_validator, sender) = create_oversized_validator(validity_start_height);
    let funding = TransactionBuilder::new_basic(
        &key_pair_with_funds(),
        sender,
        Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT + 100),
        Coin::from_u64_unchecked(100),
        validity_start_height,
        NetworkId::UnitAlbatross,
    )
    .unwrap();

    let block = temp_producer1.next_block_with_txs(vec![], false, vec![funding]);
    assert_eq!(temp_producer2.push(block), Ok(PushResult::Extended));
    let hist_tx_pre = get_hist_tx(&temp_producer2);

    // Both the block producer and a node following it store the oversized transaction
    let block = temp_producer1.next_block_with_txs(vec![], false, vec![create_validator.clone()]);
    assert_eq!(temp_producer2.push(block), Ok(PushResult::Extended));
    for temp_producer in [&temp_producer1, &temp_producer2] {
        let hist_tx = get_hist_tx(temp_producer)
            .into_iter()
            .find(|hist_tx| {
                matches!(
                    &hist_tx.data,
                    HistoricTransactionData::Basic(tx) if tx.get_raw_transaction() == &create_validator
                )
            })
            .expect("Oversized transaction must be in the history");
        assert!(size_of::<u32>() + hist_tx.serialized_size() > MAX_DUP_VALUE_SIZE);
    }

    revert_block(&temp_producer2, &hist_tx_pre);
}
