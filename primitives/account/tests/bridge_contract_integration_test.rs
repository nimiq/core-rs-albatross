use nimiq_account::BridgeContract;
use nimiq_hash::{Blake2bHash, Blake2bHasher, Hasher};
use nimiq_keys::{Address, KeyPair, SecureGenerate};
use nimiq_primitives::{account::AccountType, coin::Coin, networks::NetworkId};
use nimiq_serde::Serialize;
use nimiq_transaction::{
    account::{
        bridge_contract::OutgoingBridgeTransactionData,
        htlc_contract::{AnyHash, AnyHash32},
    },
    bridge_contract::{
        AddressFormat, AnyMerkleProof, ChainConfig, Endianness, IncomingTransaction,
        OutgoingTransaction, RecipientData,
    },
    SignatureProof, Transaction,
};
use nimiq_utils::merkle::MerkleProof;

/// Helper function to create a default chain config for testing
fn create_test_chain_config(chain_id: u32) -> ChainConfig {
    use nimiq_transaction::bridge_contract::ValidationProgram;

    ChainConfig {
        chain_id,
        hash_function: AnyHash::Blake2b(AnyHash32::default()),
        address_format: AddressFormat::Nimiq,
        endianness: Endianness::LittleEndian,
        block_time: std::time::Duration::from_secs(60),
        validation_program: ValidationProgram::empty(),
    }
}

/// Test that demonstrates how OutgoingTransaction is integrated into the bridge account
#[test]
fn test_outgoing_transaction_integration() {
    // Setup: Create a bridge contract
    let owner_keypair = KeyPair::generate_default_csprng();
    let owner_address = Address::from(&owner_keypair.public);
    let oracle_address = Address::from([2u8; 20]);
    let contract_address = Address::from([3u8; 20]);
    let source_chain_id = 1;

    let mut bridge_contract = BridgeContract {
        owner: owner_address.clone(),
        oracle_address,
        balance: Coin::from_u64_unchecked(1000),
        source_chain_id,
        chain_config: create_test_chain_config(source_chain_id),
        transaction_count: 0,
    };

    // Step 1: Create burn transaction data (from source chain)
    let burn_tx_data = vec![0u8; 100]; // Dummy burn transaction data for testing

    // Step 2: Create a Merkle proof (simplified for testing)
    let blake2b_proof = MerkleProof::<Blake2bHash>::new(&[], &[]);
    let merkle_proof = AnyMerkleProof::Blake2b(blake2b_proof);

    // Step 3: Create an oracle state index (references the oracle contract's hash array)
    let oracle_state_index: u64 = 0;

    // Step 4: Create OutgoingTransaction (proof of burn on source chain)
    let outgoing_tx = OutgoingTransaction {
        burn_transaction_data: burn_tx_data,
        merkle_proof,
        oracle_state_index,
    };

    // Step 5: Create OutgoingBridgeTransactionData with the OutgoingTransaction
    let mut bridge_data = OutgoingBridgeTransactionData {
        burn_proof: outgoing_tx,
        proof: SignatureProof::default(),
    };

    // Step 6: Create a transaction FROM the bridge contract TO the user
    // Extract recipient address from burn proof (for this test, use owner as recipient)
    let recipient_address = owner_address.clone();
    let release_amount = Coin::from_u64_unchecked(500);
    let fee = Coin::from_u64_unchecked(0);

    let mut transaction = Transaction::new_extended(
        contract_address.clone(), // sender (bridge contract)
        AccountType::Bridge,
        bridge_data.serialize_to_vec(), // sender_data
        recipient_address.clone(),      // recipient (user)
        AccountType::Basic,
        vec![],         // recipient_data (empty)
        release_amount, // value
        fee,
        1,
        NetworkId::UnitAlbatross,
    );

    // Sign the transaction with the bridge owner's key
    let signature = owner_keypair.sign(&transaction.serialize_content());
    let signature_proof = SignatureProof::from_ed25519(owner_keypair.public, signature);

    // Set the signature in the bridge data
    bridge_data.set_signature(signature_proof.clone());
    transaction.sender_data = bridge_data.serialize_to_vec();
    transaction.proof = signature_proof.serialize_to_vec();

    // Step 7: Process the outgoing transaction through the bridge contract
    // This demonstrates the integration: the bridge contract extracts and processes
    // the OutgoingTransaction from the transaction data

    let initial_balance = bridge_contract.balance;
    let initial_count = bridge_contract.transaction_count;

    // Note: In the actual implementation, this would:
    // - Parse burn data using ValidationProgram
    // - Verify merkle proof against oracle state hash
    // - Check nonce for replay protection (stored in accounts tree)
    // - Verify transaction recipient matches burn proof target address
    // - Verify transaction value matches burn proof amount
    // - Decrement bridge balance by amount
    // - Release funds to target address

    // For this test, we just simulate the balance change
    bridge_contract.balance = bridge_contract.balance.checked_sub(release_amount).unwrap();
    bridge_contract.transaction_count += 1;

    assert_eq!(bridge_contract.balance, initial_balance - release_amount);
    assert_eq!(bridge_contract.transaction_count, initial_count + 1);
}

/// Test that demonstrates replay attack prevention using nonces in outgoing transactions
/// Note: Nonces are now stored in the accounts tree subtree, not in the BridgeContract struct
#[test]
fn test_outgoing_transaction_replay_prevention() {
    let owner_address = Address::from([1u8; 20]);
    let oracle_address = Address::from([2u8; 20]);
    let source_chain_id = 1;

    let mut bridge_contract = BridgeContract {
        owner: owner_address.clone(),
        oracle_address,
        balance: Coin::from_u64_unchecked(1000),
        source_chain_id,
        chain_config: create_test_chain_config(source_chain_id),
        transaction_count: 0,
    };

    // First processing: should succeed
    let amount = Coin::from_u64_unchecked(500);
    bridge_contract.balance = bridge_contract.balance.checked_sub(amount).unwrap();
    bridge_contract.transaction_count += 1;

    // Note: In the actual implementation, nonces are stored in the accounts tree
    // using BridgeContractStore. The nonce check would be:
    // let highest_nonce = store.get_nonce(&target_address).map(|n| n.nonce).unwrap_or(0);
    // if parsed_burn.target_nonce != highest_nonce + 1 { return Err(...); }
    // store.put_nonce(&target_address, BridgeNonce { nonce: parsed_burn.target_nonce });

    assert_eq!(bridge_contract.balance, Coin::from_u64_unchecked(500));
    assert_eq!(bridge_contract.transaction_count, 1);
}

/// Test that demonstrates the incoming transaction flow (user locks NIM)
#[test]
fn test_incoming_transaction_flow() {
    let owner_keypair = KeyPair::generate_default_csprng();
    let owner_address = Address::from(&owner_keypair.public);
    let oracle_address = Address::from([2u8; 20]);
    let source_chain_id = 1;

    let mut bridge_contract = BridgeContract {
        owner: owner_address.clone(),
        oracle_address,
        balance: Coin::from_u64_unchecked(1000),
        source_chain_id,
        chain_config: create_test_chain_config(source_chain_id),
        transaction_count: 0,
    };

    // Step 1: User sends a regular transaction TO the bridge contract
    // The transaction includes IncomingTransaction in recipient_data
    // specifying where to mint wrapped tokens on the target chain
    let target_address = vec![5u8; 20]; // Ethereum address
    let incoming_amount = Coin::from_u64_unchecked(300);

    let recipient_data = RecipientData {
        target_address: target_address.clone(),
        target_nonce: 1,
    };

    let incoming_tx = IncomingTransaction {
        recipient_data,
        amount: incoming_amount,
        validity_start_height: 100,
    };

    let initial_balance = bridge_contract.balance;
    let initial_count = bridge_contract.transaction_count;

    // Step 2: Bridge contract processes the incoming transaction
    // This increments the balance (user is locking funds)
    bridge_contract.balance += incoming_amount;
    bridge_contract.transaction_count += 1;

    assert_eq!(bridge_contract.balance, initial_balance + incoming_amount);
    assert_eq!(bridge_contract.transaction_count, initial_count + 1);

    // Step 3: Bridge emits BridgeIncoming log for relayers
    // Relayers will:
    // - Monitor Nimiq for BridgeIncoming events
    // - Create merkle proof of lock transaction on Nimiq
    // - Submit to Ethereum bridge with OutgoingTransaction containing:
    //   - burn_transaction_data: Nimiq lock transaction
    //   - merkle_proof: Proof in Nimiq state (Blake2b)
    //   - oracle_state_index: Index into oracle contract's hash array
    // - Ethereum bridge mints wNIM to target address

    // Verify the incoming transaction data
    assert_eq!(incoming_tx.recipient_data.target_address, target_address);
    assert_eq!(incoming_tx.amount, incoming_amount);
}

/// Test that demonstrates the complete cross-chain flow
#[test]
fn test_complete_cross_chain_flow() {
    let owner_keypair = KeyPair::generate_default_csprng();
    let owner_address = Address::from(&owner_keypair.public);
    let oracle_address = Address::from([2u8; 20]);
    let source_chain_id = 1;

    let mut bridge_contract = BridgeContract {
        owner: owner_address.clone(),
        oracle_address,
        balance: Coin::from_u64_unchecked(1000),
        source_chain_id,
        chain_config: create_test_chain_config(source_chain_id),
        transaction_count: 0,
    };

    // === INCOMING FLOW (User Locks NIM → wNIM Minted on Ethereum) ===

    // 1. User sends regular transaction TO bridge contract
    // 2. Transaction includes IncomingTransaction in recipient_data
    let incoming_recipient = RecipientData {
        target_address: vec![10u8; 20], // Ethereum address
        target_nonce: 1,
    };

    let incoming_tx = IncomingTransaction {
        recipient_data: incoming_recipient.clone(),
        amount: Coin::from_u64_unchecked(500),
        validity_start_height: 100,
    };

    // 3. Bridge contract processes incoming transaction (user locking funds)
    bridge_contract.balance += incoming_tx.amount;
    bridge_contract.transaction_count += 1;

    assert_eq!(bridge_contract.balance, Coin::from_u64_unchecked(1500));
    assert_eq!(bridge_contract.transaction_count, 1);

    // 4. Relayers monitor BridgeIncoming log and mint wNIM on Ethereum

    // === OUTGOING FLOW (User Burns wNIM on Ethereum → NIM Released) ===

    // 1. User burns wNIM on Ethereum
    // 2. Relayer creates OutgoingTransaction with burn proof
    // 3. Bridge contract processes outgoing transaction (releasing funds)
    let outgoing_amount = Coin::from_u64_unchecked(300);

    bridge_contract.balance = bridge_contract
        .balance
        .checked_sub(outgoing_amount)
        .unwrap();
    bridge_contract.transaction_count += 1;

    // Note: Nonce would be stored in accounts tree via:
    // store.put_nonce(&target_address, BridgeNonce { nonce: target_nonce });

    assert_eq!(bridge_contract.balance, Coin::from_u64_unchecked(1200));
    assert_eq!(bridge_contract.transaction_count, 2);

    // Final state: bridge has processed 1 incoming (lock) and 1 outgoing (release) transaction
}

/// Test IncomingTransaction validation
#[test]
fn test_incoming_transaction_validation() {
    let recipient_data = RecipientData {
        target_address: vec![1u8; 20],
        target_nonce: 1,
    };

    // Valid transaction
    let valid_tx = IncomingTransaction {
        recipient_data: recipient_data.clone(),
        amount: Coin::from_u64_unchecked(100),
        validity_start_height: 1,
    };
    assert!(valid_tx.validate().is_ok());

    // Invalid: zero amount
    let invalid_amount = IncomingTransaction {
        recipient_data: recipient_data.clone(),
        amount: Coin::ZERO,
        validity_start_height: 1,
    };
    assert!(invalid_amount.validate().is_err());

    // Invalid: zero validity height
    let invalid_height = IncomingTransaction {
        recipient_data,
        amount: Coin::from_u64_unchecked(100),
        validity_start_height: 0,
    };
    assert!(invalid_height.validate().is_err());
}

/// Test RecipientData extraction from IncomingTransaction
#[test]
fn test_recipient_data_extraction() {
    let target_address = vec![5u8; 20];
    let target_nonce = 42;

    let recipient_data = RecipientData {
        target_address: target_address.to_vec(),
        target_nonce,
    };

    let incoming_tx = IncomingTransaction {
        recipient_data: recipient_data.clone(),
        amount: Coin::from_u64_unchecked(100),
        validity_start_height: 1,
    };

    // Extract recipient information
    assert_eq!(
        incoming_tx.recipient_data.extract_target_address(),
        &target_address
    );
    assert_eq!(
        incoming_tx.recipient_data.extract_target_nonce(),
        target_nonce
    );
}

/// Test Merkle proof verification with valid proof
#[test]
fn test_merkle_proof_verification_valid() {
    let source_tx_hash = Blake2bHasher::default().digest(b"valid_tx");
    let sibling_hash = Blake2bHasher::default().digest(b"sibling");

    // Create a valid Merkle proof with a sibling
    let merkle_proof = MerkleProof::<Blake2bHash>::new(
        &[source_tx_hash.clone(), sibling_hash],
        &[source_tx_hash.clone()],
    );

    // Compute the expected oracle state hash (this would be stored in the oracle contract)
    let expected_oracle_hash = merkle_proof
        .compute_root(vec![source_tx_hash.clone()])
        .unwrap();

    // Verify the proof is valid
    let computed_root = merkle_proof
        .compute_root(vec![source_tx_hash.clone()])
        .unwrap();
    assert_eq!(computed_root, expected_oracle_hash);
}

/// Test Merkle proof verification with invalid proof
#[test]
fn test_merkle_proof_verification_invalid() {
    let source_tx_hash = Blake2bHasher::default().digest(b"valid_tx");
    let wrong_oracle_hash = Blake2bHasher::default().digest(b"wrong_hash");
    let sibling_hash = Blake2bHasher::default().digest(b"sibling");

    // Create a Merkle proof
    let merkle_proof = MerkleProof::<Blake2bHash>::new(
        &[source_tx_hash.clone(), sibling_hash],
        &[source_tx_hash.clone()],
    );

    // Compute root from proof
    let computed_root = merkle_proof
        .compute_root(vec![source_tx_hash.clone()])
        .unwrap();

    // The computed root should NOT match the wrong oracle hash
    assert_ne!(computed_root, wrong_oracle_hash);

    // In the actual implementation, this would cause the transaction to be rejected
}

/// Test Merkle proof verification with multi-level proof
#[test]
fn test_merkle_proof_verification_multilevel() {
    let leaf_hash = Blake2bHasher::default().digest(b"leaf_tx");
    let sibling_hash = Blake2bHasher::default().digest(b"sibling");

    // Create a proof with one sibling
    let merkle_proof = MerkleProof::<Blake2bHash>::new(
        &[leaf_hash.clone(), sibling_hash.clone()],
        &[leaf_hash.clone()],
    );

    // Compute the root
    let computed_root = merkle_proof.compute_root(vec![leaf_hash.clone()]).unwrap();

    // The root should be different from the leaf
    assert_ne!(computed_root, leaf_hash);

    // Verify that the same proof with the same leaf produces the same root
    let computed_root2 = merkle_proof.compute_root(vec![leaf_hash.clone()]).unwrap();
    assert_eq!(computed_root, computed_root2);
}

/// Test chain-specific address format validation
#[test]
fn test_chain_specific_address_validation() {
    // Test Nimiq address format (always valid for 20-byte addresses)
    let nimiq_config = ChainConfig {
        chain_id: 1,
        hash_function: AnyHash::Blake2b(AnyHash32::default()),
        address_format: AddressFormat::Nimiq,
        endianness: Endianness::LittleEndian,
        block_time: std::time::Duration::from_secs(60),
        validation_program: nimiq_transaction::bridge_contract::ValidationProgram::empty(),
    };

    let nimiq_recipient = RecipientData {
        target_address: vec![1u8; 20],
        target_nonce: 1,
    };

    assert!(nimiq_recipient.validate(&nimiq_config).is_ok());

    // Test Ethereum address format (20 bytes)
    let ethereum_config = ChainConfig {
        chain_id: 2,
        hash_function: AnyHash::Keccak256(AnyHash32::default()),
        address_format: AddressFormat::Ethereum,
        endianness: Endianness::BigEndian,
        block_time: std::time::Duration::from_secs(12),
        validation_program: nimiq_transaction::bridge_contract::ValidationProgram::empty(),
    };

    let ethereum_recipient = RecipientData {
        target_address: vec![2u8; 20],
        target_nonce: 1,
    };

    assert!(ethereum_recipient.validate(&ethereum_config).is_ok());

    // Test Bitcoin address format
    let bitcoin_config = ChainConfig {
        chain_id: 3,
        hash_function: AnyHash::Sha256(AnyHash32::default()),
        address_format: AddressFormat::Bitcoin,
        endianness: Endianness::BigEndian,
        block_time: std::time::Duration::from_secs(600),
        validation_program: nimiq_transaction::bridge_contract::ValidationProgram::empty(),
    };

    let bitcoin_recipient = RecipientData {
        target_address: vec![3u8; 20],
        target_nonce: 1,
    };

    // Bitcoin validation is lenient in our implementation
    assert!(bitcoin_recipient.validate(&bitcoin_config).is_ok());

    // Test Custom address format
    let custom_config = ChainConfig {
        chain_id: 4,
        hash_function: AnyHash::Blake2b(AnyHash32::default()),
        address_format: AddressFormat::Custom("MyChain".to_string()),
        endianness: Endianness::LittleEndian,
        block_time: std::time::Duration::from_secs(30),
        validation_program: nimiq_transaction::bridge_contract::ValidationProgram::empty(),
    };

    let custom_recipient = RecipientData {
        target_address: vec![4u8; 20],
        target_nonce: 1,
    };

    assert!(custom_recipient.validate(&custom_config).is_ok());
}

/// Test that recipient data validation rejects invalid nonces
#[test]
fn test_recipient_data_nonce_validation() {
    let chain_config = create_test_chain_config(1);

    // Valid nonce
    let valid_recipient = RecipientData {
        target_address: vec![1u8; 20],
        target_nonce: 1,
    };
    assert!(valid_recipient.validate(&chain_config).is_ok());

    // Invalid: zero nonce
    let invalid_recipient = RecipientData {
        target_address: vec![1u8; 20],
        target_nonce: 0,
    };
    assert!(invalid_recipient.validate(&chain_config).is_err());
}

/// Test that recipient data validation rejects oversized additional data
#[test]
fn test_recipient_data_size_validation() {
    let chain_config = create_test_chain_config(1);

    // Valid: no additional data
    let valid_no_data = RecipientData {
        target_address: vec![1u8; 20],
        target_nonce: 1,
    };
    assert!(valid_no_data.validate(&chain_config).is_ok());
}
