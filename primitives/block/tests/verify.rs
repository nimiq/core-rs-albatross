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
    policy::{upgrades, Policy},
    slots_allocation::{Validator, Validators, ValidatorsBuilder},
};
use nimiq_serde::Deserialize;
use nimiq_test_log::test;
use nimiq_test_utils::{
    blockchain::{generate_transactions, validator_address},
    versions::{assert_all_versions, bp},
};
use nimiq_transaction::{ExecutedTransaction, Transaction};

/// Test blocks use timestamp 0; this allows up to 1 second into the future.
const TEST_MAX_TIMESTAMP: u64 = 1000;

/// A real micro block produced under a protocol version below `DIFF_ROOT_COMMITMENT`.
const PRE_COMMITMENT_VERSION_MICRO_BLOCK: &str = "010701c90188cfe981cd2f4f8f18fcfe98ccacee0745b6aa69365cd28c836f962d577de57f04aa17fe54a18bb8c886fe96c5cff2a6a2f5e5587779c2a08b3bdccb68923aa30879b5169b238a0a6251c5a0e64a0809bd296d1bf1e9f9c75f07d546ce5b6f511d5da932e301a48c1c89b89fb46ac24d3e190fb72bd17d268407d36a48cd73ced4e870402e07006dd08f34183d089f3e95917e05ebdc11983aa6f433d99332af0cb74b0e299da2774f9018b4b2cdc08f8928e89b0d963ac388e537b003492c69aafeeab976e55303170a2e7597b7b7e3d84c05391d139a62b157e78786d8c082f29dcf4c11131481e47a19e6b29b0a65b9591762ce5143ed30d0261e5d24a3201752506b20f15c0100e873f12b712f8fe1c337fefb0bacd37dc9257846878386e71e15e05ce4f6fbc31b414ce0887c6a03dd34a36a40993c44aa11878883c0df683a1b415f47fa7a02010000";

/// A real macro block produced under a protocol version below `DIFF_ROOT_COMMITMENT`.
const PRE_COMMITMENT_VERSION_MACRO_BLOCK: &str = "000701e80100b8b9eb81cd2f2a59f22014aa9b1bf739abf71dbdb02eaa954e4febbc8fd4ffc43356e531e3804f8f18fcfe98ccacee0745b6aa69365cd28c836f962d577de57f04aa17fe54a1008090c81856fb3385f88ab330fbb222a1f776186011eab09980705585271ea04571c4042a747bfb7e3b4f089b0109968c9a2359abf18d8aa31258b9d8af68bf0f2aa3ae2fb99b75e24684d336cf50371888556cac228ec10c1ab32499df0b3505006dd08f34183d089f3e95917e05ebdc11983aa6f433d99332af0cb74b0e299da24f3ac8d1e747d2f46da2fef81f51cad8be2664acec4b30926083d11b7c811c6c03170a2e7597b7b7e3d84c05391d139a62b157e78786d8c082f29dcf4c11131481e47a19e6b29b0a65b9591762ce5143ed30d0261e5d24a3201752506b20f15c000001000100a0caba01ff7b00f356b6e20ba6a297f2d7901042c460b4cb2502bb3478fcd8397ca6fc5fb28361938021b5fe69412074e646f17853dec44a17fb245c82f016f46c6068886a16fcf6e9da19280db7ed3d4632271ce4983970ec376ddd04258008ffffffffffffffffff01ffffffffffffffffff01ffffffffffffffffff01ffffffffffffffffff01ffffffffffffffffff01ffffffffffffffffff01ffffffffffffffffff01ffffffffffffffffff01";

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

/// Builds a header at `version` and reports whether changing only `diff_root` changes the header
/// hash, i.e. whether the hash commits to `diff_root`.
fn micro_header_hash_commits_to_diff_root(version: u16) -> bool {
    let mut header = MicroHeader {
        network: NetworkId::UnitAlbatross,
        version,
        block_number: 1,
        timestamp: 0,
        ..Default::default()
    };
    let header_hash = header.hash();
    header.diff_root = "a different diff root".hash();
    header_hash != header.hash()
}

fn macro_header_hash_commits_to_diff_root(version: u16) -> bool {
    let mut header = MacroHeader {
        network: NetworkId::UnitAlbatross,
        version,
        block_number: Policy::macro_block_after(1),
        round: 0,
        timestamp: 0,
        ..Default::default()
    };
    let header_hash = header.hash();
    header.diff_root = "a different diff root".hash();
    header_hash != header.hash()
}

/// Micro block header hashes are not gated: they commit to `diff_root` in every version.
#[test]
fn it_commits_to_the_diff_root_in_the_micro_header_hash() {
    assert_all_versions(micro_header_hash_commits_to_diff_root, vec![]);
}

/// The macro block header hash commits to `diff_root` from `DIFF_ROOT_COMMITMENT` onward;
/// before that version it does not (so pre-commitment-version hashes stay unchanged).
#[test]
fn it_commits_to_the_diff_root_in_the_macro_header_hash_from_the_commitment_version() {
    assert_all_versions(
        |v| !macro_header_hash_commits_to_diff_root(v),
        vec![bp(
            upgrades::v2::DIFF_ROOT_COMMITMENT,
            macro_header_hash_commits_to_diff_root,
        )],
    );
}

/// A real micro block from before `DIFF_ROOT_COMMITMENT` must keep its original hash: the
/// gating must not retroactively change the hash of pre-commitment-version blocks.
#[test]
fn it_preserves_the_hash_of_a_pre_commitment_version_micro_block() {
    let block =
        Block::deserialize_from_vec(&hex::decode(PRE_COMMITMENT_VERSION_MICRO_BLOCK).unwrap())
            .unwrap();

    assert!(block.version() < upgrades::v2::DIFF_ROOT_COMMITMENT);
    assert_eq!(
        block.hash().to_string(),
        "79ec03e9fc624a9ebffe66bc724349cf993104a24d6296bb13240174488046cb",
    );
}

/// A real macro block from before `DIFF_ROOT_COMMITMENT` must keep its original hash: the
/// gating must not retroactively change the hash of pre-commitment-version blocks.
#[test]
fn it_preserves_the_hash_of_a_pre_commitment_version_macro_block() {
    let block =
        Block::deserialize_from_vec(&hex::decode(PRE_COMMITMENT_VERSION_MACRO_BLOCK).unwrap())
            .unwrap();

    assert!(block.version() < upgrades::v2::DIFF_ROOT_COMMITMENT);
    assert_eq!(
        block.hash().to_string(),
        "4aa3e0d454336e6dd28656318fd3290a62f91b5c66b4e5c5cc05646c9f58eb39",
    );
}
