use ark_ec::PrimeGroup;
use nimiq_block::{
    Block, BlockError, EquivocationProof, ForkProof, MacroBlock, MacroBody, MacroHeader,
    MicroBlock, MicroBody, MicroHeader, MicroJustification, MultiSignature, SkipBlockProof,
};
use nimiq_bls::{AggregateSignature, G2Projective, PublicKey as BlsPublicKey};
use nimiq_collections::BitSet;
use nimiq_hash::Hash;
use nimiq_keys::{Address, Ed25519PublicKey as SchnorrPublicKey, Ed25519Signature, KeyPair};
use nimiq_primitives::{
    coin::Coin,
    networks::NetworkId,
    policy::Policy,
    slots_allocation::{Validator, Validators, ValidatorsBuilder},
};
use nimiq_test_log::test;
use nimiq_test_utils::blockchain::{generate_transactions, validator_address};
use nimiq_transaction::{ExecutedTransaction, Transaction};

/// Test blocks use timestamp 0; this allows up to 1 second into the future.
const TEST_MAX_TIMESTAMP: u64 = 1000;

#[test]
fn test_verify_header_network() {
    let mut block = Block::Micro(MicroBlock {
        header: MicroHeader {
            network: NetworkId::DevAlbatross,
            version: Policy::max_supported_version(),
            block_number: 1,
            timestamp: 0,
            ..Default::default()
        },
        justification: None,
        body: None,
    });

    // Check version at header level
    assert_eq!(
        block.verify_header(NetworkId::UnitAlbatross, false, TEST_MAX_TIMESTAMP),
        Err(BlockError::NetworkMismatch)
    );

    // Error should remain at block level
    assert_eq!(
        block.verify(NetworkId::UnitAlbatross, TEST_MAX_TIMESTAMP),
        Err(BlockError::NetworkMismatch)
    );

    // Fix the version and check that it passes
    block.unwrap_micro_ref_mut().header.network = NetworkId::UnitAlbatross;
    assert_eq!(
        block.verify_header(NetworkId::UnitAlbatross, false, TEST_MAX_TIMESTAMP),
        Ok(())
    );
}

#[test]
fn test_verify_header_version() {
    let mut block = Block::Micro(MicroBlock {
        header: MicroHeader {
            network: NetworkId::UnitAlbatross,
            version: Policy::max_supported_version() + 1,
            block_number: 1,
            timestamp: 0,
            ..Default::default()
        },
        justification: None,
        body: None,
    });

    // Check version at header level
    assert_eq!(
        block.verify_header(NetworkId::UnitAlbatross, false, TEST_MAX_TIMESTAMP),
        Err(BlockError::UnsupportedVersion)
    );

    // Error should remain at block level
    assert_eq!(
        block.verify(NetworkId::UnitAlbatross, TEST_MAX_TIMESTAMP),
        Err(BlockError::UnsupportedVersion)
    );

    // Fix the version and check that it passes
    block.unwrap_micro_ref_mut().header.version = Policy::max_supported_version();
    assert_eq!(
        block.verify_header(NetworkId::UnitAlbatross, false, TEST_MAX_TIMESTAMP),
        Ok(())
    );
}

#[test]
fn test_verify_version_upgrades_micro_blocks() {
    // Version upgrades in subsequent micro blocks are not allowed
    let block1 = Block::Micro(MicroBlock {
        header: MicroHeader {
            network: NetworkId::UnitAlbatross,
            version: Policy::max_supported_version() - 1,
            block_number: 1,
            timestamp: 0,
            ..Default::default()
        },
        justification: None,
        body: None,
    });
    let mut block2 = Block::Micro(MicroBlock {
        header: MicroHeader {
            network: NetworkId::UnitAlbatross,
            version: Policy::max_supported_version(),
            block_number: 2,
            timestamp: 1,
            parent_hash: block1.hash(),
            ..Default::default()
        },
        justification: None,
        body: None,
    });

    // Should not allow version changes in micro blocks
    assert_eq!(
        block1.verify_header(NetworkId::UnitAlbatross, false, TEST_MAX_TIMESTAMP),
        Ok(()),
        "Should accept lower versions"
    );
    assert_eq!(
        block2.verify_header(NetworkId::UnitAlbatross, false, TEST_MAX_TIMESTAMP),
        Ok(()),
        "Should accept max version"
    );
    assert_eq!(
        block2.verify_immediate_successor(&block1),
        Err(BlockError::UnsupportedVersion),
        "Should not accept changes in micro block"
    );

    // Fix the version and check that it passes
    block2.unwrap_micro_ref_mut().header.version = Policy::max_supported_version() - 1;
    assert_eq!(
        block2.verify_header(NetworkId::UnitAlbatross, false, TEST_MAX_TIMESTAMP),
        Ok(()),
        "Should accept lower versions"
    );
    assert_eq!(
        block2.verify_immediate_successor(&block1),
        Ok(()),
        "Should accept same version"
    );
}

#[test]
fn test_verify_version_upgrades_macro_blocks() {
    // Scenario 1: Version upgrades in non-election blocks are not allowed
    let validators = Validators::new(vec![Validator::new(
        Default::default(),
        BlsPublicKey {
            public_key: Default::default(),
        },
        SchnorrPublicKey::default(),
        0..Policy::SLOTS,
    )]);
    let macro_block1 = Block::Macro(MacroBlock {
        header: MacroHeader {
            network: NetworkId::UnitAlbatross,
            version: Policy::max_supported_version() - 2,
            block_number: Policy::macro_block_after(1),
            timestamp: 0,
            validators: Some(validators.clone()),
            ..Default::default()
        },
        justification: None,
        body: None,
    });
    let mut macro_block2 = Block::Macro(MacroBlock {
        header: MacroHeader {
            network: NetworkId::UnitAlbatross,
            version: Policy::max_supported_version(),
            block_number: Policy::macro_block_after(macro_block1.block_number()),
            timestamp: 1,
            parent_election_hash: macro_block1.hash(),
            ..Default::default()
        },
        justification: None,
        body: None,
    });
    let mut election_block = Block::Macro(MacroBlock {
        header: MacroHeader {
            network: NetworkId::UnitAlbatross,
            version: Policy::max_supported_version(),
            block_number: Policy::election_block_after(macro_block2.block_number()),
            timestamp: 3,
            validators: Some(validators),
            parent_election_hash: macro_block1.hash(),
            ..Default::default()
        },
        justification: None,
        body: None,
    });

    // Should not allow version changes in non-election blocks
    assert_eq!(
        macro_block1.verify_header(NetworkId::UnitAlbatross, false, TEST_MAX_TIMESTAMP),
        Ok(()),
        "Should accept lower versions"
    );
    assert_eq!(
        macro_block2.verify_header(NetworkId::UnitAlbatross, false, TEST_MAX_TIMESTAMP),
        Ok(()),
        "Should accept max version"
    );
    assert_eq!(
        macro_block2.verify_macro_successor(macro_block1.unwrap_macro_ref()),
        Err(BlockError::UnsupportedVersion),
        "Should not accept changes in non-election block"
    );

    // Fix the version and check that it passes
    macro_block2.unwrap_macro_ref_mut().header.version = Policy::max_supported_version() - 2;
    assert_eq!(
        macro_block2.verify_header(NetworkId::UnitAlbatross, false, TEST_MAX_TIMESTAMP),
        Ok(()),
        "Should accept lower versions"
    );
    assert_eq!(
        macro_block2.verify_macro_successor(macro_block1.unwrap_macro_ref()),
        Ok(()),
        "Should accept same version"
    );

    // Scenario 2: Version upgrades above one in election blocks are not allowed
    // Should not allow version changes > 1
    assert_eq!(
        election_block.verify_header(NetworkId::UnitAlbatross, false, TEST_MAX_TIMESTAMP),
        Ok(()),
        "Should accept max versions"
    );
    assert_eq!(
        election_block.verify_macro_successor(macro_block2.unwrap_macro_ref()),
        Err(BlockError::UnsupportedVersion),
        "Should not accept version upgrades > 1"
    );

    // Scenario 3: Version upgrades in election blocks are allowed
    // Fix the version and check that it passes
    election_block.unwrap_macro_ref_mut().header.version = Policy::max_supported_version() - 1;
    assert_eq!(
        election_block.verify_header(NetworkId::UnitAlbatross, false, TEST_MAX_TIMESTAMP),
        Ok(()),
        "Should accept lower versions"
    );
    assert_eq!(
        election_block.verify_macro_successor(macro_block2.unwrap_macro_ref()),
        Ok(()),
        "Should accept version upgrade"
    );
}

#[test]
fn test_verify_header_extra_data() {
    let mut block = Block::Micro(MicroBlock {
        header: MicroHeader {
            network: NetworkId::UnitAlbatross,
            version: Policy::max_supported_version(),
            block_number: 1,
            timestamp: 0,
            extra_data: vec![0; 33],
            ..Default::default()
        },
        justification: None,
        body: None,
    });

    // Check extra data field at header level
    assert_eq!(
        block.verify_header(NetworkId::UnitAlbatross, false, TEST_MAX_TIMESTAMP),
        Err(BlockError::ExtraDataTooLarge)
    );
    // Error should remain for a skip block
    assert_eq!(
        block.verify_header(NetworkId::UnitAlbatross, true, TEST_MAX_TIMESTAMP),
        Err(BlockError::ExtraDataTooLarge)
    );

    // Error should remain at block level
    assert_eq!(
        block.verify(NetworkId::UnitAlbatross, TEST_MAX_TIMESTAMP),
        Err(BlockError::ExtraDataTooLarge)
    );

    // Fix the extra data field and check that it passes
    block.unwrap_micro_ref_mut().header.extra_data = vec![0; 32];
    assert_eq!(
        block.verify_header(NetworkId::UnitAlbatross, false, TEST_MAX_TIMESTAMP),
        Ok(())
    );
    // Error should remain for a skip block
    assert_eq!(
        block.verify_header(NetworkId::UnitAlbatross, true, TEST_MAX_TIMESTAMP),
        Err(BlockError::ExtraDataTooLarge)
    );

    // Fix the extra data field for a skip block and check that it passes
    block.unwrap_micro_ref_mut().header.extra_data = [].to_vec();
    assert_eq!(
        block.verify_header(NetworkId::UnitAlbatross, true, TEST_MAX_TIMESTAMP),
        Ok(())
    );
}

#[test]
fn test_verify_body_root() {
    let mut micro_header = MicroHeader {
        network: NetworkId::UnitAlbatross,
        version: Policy::max_supported_version(),
        block_number: 1,
        timestamp: 0,
        extra_data: vec![0; 30],
        ..Default::default()
    };

    let micro_justification = MicroJustification::Micro(Ed25519Signature::default());

    let micro_body = MicroBody {
        equivocation_proofs: [].to_vec(),
        transactions: [].to_vec(),
    };

    // Build a block with body
    let block = Block::Micro(MicroBlock {
        header: micro_header.clone(),
        justification: Some(micro_justification.clone()),
        body: Some(micro_body.clone()),
    });

    // The body root check must fail
    assert_eq!(
        block.verify(NetworkId::UnitAlbatross, TEST_MAX_TIMESTAMP),
        Err(BlockError::BodyHashMismatch)
    );

    // Fix the body root and check that it passes
    micro_header.body_root = micro_body.hash();
    let block = Block::Micro(MicroBlock {
        header: micro_header,
        justification: Some(micro_justification),
        body: Some(micro_body),
    });

    assert_eq!(
        block.verify(NetworkId::UnitAlbatross, TEST_MAX_TIMESTAMP),
        Ok(())
    );
}

#[test]
fn test_verify_skip_block() {
    let mut micro_header = MicroHeader {
        network: NetworkId::UnitAlbatross,
        version: Policy::max_supported_version(),
        block_number: 1,
        timestamp: 0,
        ..Default::default()
    };

    let micro_justification = MicroJustification::Skip(SkipBlockProof {
        sig: MultiSignature {
            signature: AggregateSignature::new(),
            signers: BitSet::default(),
        },
    });

    let transactions: Vec<ExecutedTransaction> =
        generate_transactions(&KeyPair::default(), 1, NetworkId::UnitAlbatross, 1, 0)
            .iter()
            .map(|tx| ExecutedTransaction::Ok(tx.clone()))
            .collect();

    let mut micro_body = MicroBody {
        equivocation_proofs: vec![],
        transactions,
    };

    // Build a block with body
    micro_header.body_root = micro_body.hash();
    let block = Block::Micro(MicroBlock {
        header: micro_header.clone(),
        justification: Some(micro_justification.clone()),
        body: Some(micro_body.clone()),
    });

    // The skip block body should fail
    assert_eq!(
        block.verify(NetworkId::UnitAlbatross, TEST_MAX_TIMESTAMP),
        Err(BlockError::InvalidSkipBlockBody)
    );

    // Fix the body with empty transactions and check that it passes
    micro_body.transactions = vec![];
    micro_header.body_root = micro_body.hash();
    let block = Block::Micro(MicroBlock {
        header: micro_header,
        justification: Some(micro_justification),
        body: Some(micro_body),
    });

    assert_eq!(
        block.verify(NetworkId::UnitAlbatross, TEST_MAX_TIMESTAMP),
        Ok(())
    );
}

#[test]
fn test_verify_micro_block_body_txns() {
    let mut micro_header = MicroHeader {
        network: NetworkId::UnitAlbatross,
        version: Policy::max_supported_version(),
        block_number: 1,
        timestamp: 0,
        ..Default::default()
    };

    let micro_justification = MicroJustification::Micro(Ed25519Signature::default());

    let txns: Vec<ExecutedTransaction> =
        generate_transactions(&KeyPair::default(), 1, NetworkId::UnitAlbatross, 5, 0)
            .iter()
            .map(|tx| ExecutedTransaction::Ok(tx.clone()))
            .collect();

    // Lets have a duplicate transaction
    let mut txns_dup = txns.clone();
    txns_dup.push(txns.first().unwrap().clone());

    let mut micro_body = MicroBody {
        equivocation_proofs: [].to_vec(),
        transactions: txns_dup.clone(),
    };

    // Build a block with body with a duplicate transaction
    micro_header.body_root = micro_body.hash();
    let block = Block::Micro(MicroBlock {
        header: micro_header.clone(),
        justification: Some(micro_justification.clone()),
        body: Some(micro_body.clone()),
    });

    // The body check should fail
    assert_eq!(
        block.verify(NetworkId::UnitAlbatross, TEST_MAX_TIMESTAMP),
        Err(BlockError::DuplicateTransaction)
    );

    // Fix the body with empty transactions and check that it passes
    txns_dup.pop();
    micro_body.transactions = txns_dup;
    micro_header.body_root = micro_body.hash();
    let block = Block::Micro(MicroBlock {
        header: micro_header.clone(),
        justification: Some(micro_justification.clone()),
        body: Some(micro_body),
    });

    assert_eq!(
        block.verify(NetworkId::UnitAlbatross, TEST_MAX_TIMESTAMP),
        Ok(())
    );

    // Now modify the validity start height
    let txns: Vec<ExecutedTransaction> = generate_transactions(
        &KeyPair::default(),
        Policy::blocks_per_epoch(),
        NetworkId::UnitAlbatross,
        5,
        0,
    )
    .iter()
    .map(|tx| ExecutedTransaction::Ok(tx.clone()))
    .collect();

    let micro_body = MicroBody {
        equivocation_proofs: vec![],
        transactions: txns,
    };

    // Build a block with body with the expired transactions
    micro_header.body_root = micro_body.hash();
    let block = Block::Micro(MicroBlock {
        header: micro_header,
        justification: Some(micro_justification),
        body: Some(micro_body),
    });

    // The body check should fail
    assert_eq!(
        block.verify(NetworkId::UnitAlbatross, TEST_MAX_TIMESTAMP),
        Err(BlockError::ExpiredTransaction)
    );
}

#[test]
fn test_verify_micro_block_body_fork_proofs() {
    let genesis_block_number = Policy::genesis_block_number();
    let mut micro_header = MicroHeader {
        network: NetworkId::UnitAlbatross,
        version: Policy::max_supported_version(),
        block_number: 1 + genesis_block_number,
        timestamp: 0,
        ..Default::default()
    };

    let mut micro_header_1 = micro_header.clone();
    let mut micro_header_2 = micro_header.clone();
    micro_header_2.timestamp += 1;

    let fork_proof_1 = ForkProof::new(
        validator_address(),
        micro_header_1.clone(),
        Ed25519Signature::default(),
        micro_header_2.clone(),
        Ed25519Signature::default(),
    );

    micro_header_1.block_number += 1;
    micro_header_2.block_number += 1;

    let fork_proof_2 = ForkProof::new(
        validator_address(),
        micro_header_1.clone(),
        Ed25519Signature::default(),
        micro_header_2.clone(),
        Ed25519Signature::default(),
    );

    micro_header_1.block_number += 1;
    micro_header_2.block_number += 1;

    let fork_proof_3 = ForkProof::new(
        validator_address(),
        micro_header_1.clone(),
        Ed25519Signature::default(),
        micro_header_2.clone(),
        Ed25519Signature::default(),
    );

    let micro_justification = MicroJustification::Micro(Ed25519Signature::default());

    let mut fork_proofs = vec![fork_proof_1, fork_proof_2, fork_proof_3];
    fork_proofs.sort_by_key(|p| EquivocationProof::from(p.clone()).sort_key());
    fork_proofs.reverse();
    let micro_body = MicroBody {
        equivocation_proofs: fork_proofs.iter().cloned().map(Into::into).collect(),
        transactions: [].to_vec(),
    };

    // Build a block with body with a the unsorted fork proofs
    micro_header.body_root = micro_body.hash();
    let block = Block::Micro(MicroBlock {
        header: micro_header.clone(),
        justification: Some(micro_justification.clone()),
        body: Some(micro_body),
    });

    // The body check should fail
    assert_eq!(
        block.verify(NetworkId::UnitAlbatross, TEST_MAX_TIMESTAMP),
        Err(BlockError::ForkProofsNotOrdered)
    );

    // Sort fork proofs and re-build block
    fork_proofs.sort_by_key(|p| EquivocationProof::from(p.clone()).sort_key());
    let micro_body = MicroBody {
        equivocation_proofs: fork_proofs.iter().cloned().map(Into::into).collect(),
        transactions: [].to_vec(),
    };

    micro_header.body_root = micro_body.hash();
    let block = Block::Micro(MicroBlock {
        header: micro_header.clone(),
        justification: Some(micro_justification.clone()),
        body: Some(micro_body),
    });

    assert_eq!(
        block.verify(NetworkId::UnitAlbatross, TEST_MAX_TIMESTAMP),
        Ok(())
    );

    // Lets have a duplicate fork proof
    fork_proofs.push(fork_proofs.last().unwrap().clone());
    let micro_body = MicroBody {
        equivocation_proofs: fork_proofs.iter().cloned().map(Into::into).collect(),
        transactions: [].to_vec(),
    };

    micro_header.body_root = micro_body.hash();
    let block = Block::Micro(MicroBlock {
        header: micro_header.clone(),
        justification: Some(micro_justification.clone()),
        body: Some(micro_body),
    });

    assert_eq!(
        block.verify(NetworkId::UnitAlbatross, TEST_MAX_TIMESTAMP),
        Err(BlockError::DuplicateForkProof)
    );

    // Now modify the block height of the first header of the first fork proof
    let mut micro_header_large_block_number_1 = micro_header.clone();
    micro_header_large_block_number_1.block_number =
        Policy::blocks_per_epoch() + genesis_block_number;
    let mut micro_header_large_block_number_2 = micro_header_large_block_number_1.clone();
    micro_header_large_block_number_2.timestamp += 1;
    fork_proofs.pop().unwrap();
    fork_proofs.push(ForkProof::new(
        validator_address(),
        micro_header_large_block_number_1,
        Ed25519Signature::default(),
        micro_header_large_block_number_2,
        Ed25519Signature::default(),
    ));
    fork_proofs.sort_by_key(|p| EquivocationProof::from(p.clone()).sort_key());

    let micro_body = MicroBody {
        equivocation_proofs: fork_proofs.iter().cloned().map(Into::into).collect(),
        transactions: [].to_vec(),
    };

    micro_header.body_root = micro_body.hash();
    let block = Block::Micro(MicroBlock {
        header: micro_header,
        justification: Some(micro_justification),
        body: Some(micro_body),
    });

    // The first fork proof should no longer be valid
    assert_eq!(
        block.verify(NetworkId::UnitAlbatross, TEST_MAX_TIMESTAMP),
        Err(BlockError::InvalidForkProof)
    );
}

/// Helper to create a transaction with a specific fee and a unique recipient.
/// The transaction is not signed and won't pass signature verification, but is
/// sufficient for fee sum tests.
fn tx_with_fee(fee: Coin, unique_byte: u8) -> ExecutedTransaction {
    ExecutedTransaction::Ok(Transaction::new_basic(
        Address::default(),
        Address::from([unique_byte; 20]),
        Coin::from_u64_unchecked(1),
        fee,
        1,
        NetworkId::UnitAlbatross,
    ))
}

#[test]
fn test_sum_transaction_fees_overflow() {
    let high_fee = Coin::from_u64_unchecked(Policy::TOTAL_SUPPLY);

    // 5 transactions each with fee = TOTAL_SUPPLY should overflow Coin::MAX
    let transactions: Vec<ExecutedTransaction> =
        (0..5).map(|i| tx_with_fee(high_fee, i + 1)).collect();

    let micro_body = MicroBody {
        equivocation_proofs: vec![],
        transactions,
    };

    let block = Block::Micro(MicroBlock {
        header: MicroHeader {
            network: NetworkId::UnitAlbatross,
            version: Policy::max_supported_version(),
            block_number: 1,
            timestamp: 0,
            body_root: micro_body.hash(),
            ..Default::default()
        },
        justification: Some(MicroJustification::Micro(Ed25519Signature::default())),
        body: Some(micro_body),
    });

    assert_eq!(
        block.sum_transaction_fees(),
        Err(BlockError::TransactionFeeOverflow),
    );
}

#[test]
fn test_sum_transaction_fees_normal() {
    let fee = Coin::from_u64_unchecked(1000);
    let transactions: Vec<ExecutedTransaction> = (0..3).map(|i| tx_with_fee(fee, i + 1)).collect();

    let block = Block::Micro(MicroBlock {
        header: MicroHeader::default(),
        justification: None,
        body: Some(MicroBody {
            equivocation_proofs: vec![],
            transactions,
        }),
    });

    assert_eq!(
        block.sum_transaction_fees(),
        Ok(Coin::from_u64_unchecked(3000)),
    );
}

#[test]
fn test_verify_election_macro_body() {
    let mut macro_header = MacroHeader {
        network: NetworkId::UnitAlbatross,
        version: Policy::max_supported_version(),
        block_number: Policy::genesis_block_number() + Policy::blocks_per_epoch(),
        round: 0,
        timestamp: 0,
        interlink: Some(vec![]),
        extra_data: vec![0; 30],
        ..Default::default()
    };

    let macro_body = MacroBody {
        transactions: vec![],
    };
    macro_header.body_root = macro_body.hash();

    // Build an election macro block
    let block = Block::Macro(MacroBlock {
        header: macro_header.clone(),
        justification: None,
        body: Some(macro_body.clone()),
    });

    // The validators check should fail
    assert_eq!(
        block.verify(NetworkId::UnitAlbatross, TEST_MAX_TIMESTAMP),
        Err(BlockError::InvalidValidators)
    );

    // Fix the validators set
    let mut validators = ValidatorsBuilder::new();
    for _ in 0..Policy::SLOTS {
        validators.push(
            Address::default(),
            BlsPublicKey::new(G2Projective::generator()).compress(),
            SchnorrPublicKey::default(),
        );
    }
    macro_header.validators = Some(validators.build());

    macro_header.body_root = macro_body.hash();
    let block = Block::Macro(MacroBlock {
        header: macro_header,
        justification: None,
        body: Some(macro_body),
    });

    // Skipping the verification of the PK tree root should make the verify function to pass
    assert_eq!(
        block.verify(NetworkId::UnitAlbatross, TEST_MAX_TIMESTAMP),
        Ok(())
    );
}

#[test]
fn test_verify_rejects_out_of_range_punished_set() {
    // Build a valid election macro header so the punished-set check is the only thing that can fail.
    let mut validators = ValidatorsBuilder::new();
    for _ in 0..Policy::SLOTS {
        validators.push(
            Address::default(),
            BlsPublicKey::new(G2Projective::generator()).compress(),
            SchnorrPublicKey::default(),
        );
    }

    let mut macro_header = MacroHeader {
        network: NetworkId::UnitAlbatross,
        version: Policy::max_supported_version(),
        block_number: Policy::genesis_block_number() + Policy::blocks_per_epoch(),
        round: 0,
        timestamp: 0,
        interlink: Some(vec![]),
        extra_data: vec![0; 30],
        validators: Some(validators.build()),
        ..Default::default()
    };

    let macro_body = MacroBody {
        transactions: vec![],
    };
    macro_header.body_root = macro_body.hash();

    // A punished set that references an out-of-range slot (index == SLOTS) must be rejected. It
    // could otherwise disable every in-range slot while evading the all-disabled fast path in
    // `compute_slot_number` and crash light/pico clients with a division by zero
    let mut punished_set = BitSet::new();
    punished_set.insert(Policy::SLOTS as usize);
    macro_header.next_batch_initial_punished_set = punished_set;

    let block = Block::Macro(MacroBlock {
        header: macro_header.clone(),
        justification: None,
        body: Some(macro_body.clone()),
    });
    assert_eq!(
        block.verify(NetworkId::UnitAlbatross, TEST_MAX_TIMESTAMP),
        Err(BlockError::InvalidPunishedSet)
    );

    // An in-range punished set (including the boundary slot SLOTS - 1) is accepted.
    let mut punished_set = BitSet::new();
    punished_set.insert(0);
    punished_set.insert(Policy::SLOTS as usize - 1);
    macro_header.next_batch_initial_punished_set = punished_set;

    let block = Block::Macro(MacroBlock {
        header: macro_header,
        justification: None,
        body: Some(macro_body),
    });
    assert_eq!(
        block.verify(NetworkId::UnitAlbatross, TEST_MAX_TIMESTAMP),
        Ok(())
    );
}
