use beserial::{Serialize, SerializingError, Deserialize};
use nimiq::consensus::base::account::{AccountError, AccountType, VestingContract};
use nimiq::consensus::base::primitive::{Address, Coin, crypto::KeyPair};
use nimiq::consensus::base::transaction::{Transaction, TransactionError, TransactionFlags, SignatureProof};
use nimiq::consensus::networks::NetworkId;

const CONTRACT: &str = "00002fbf9bd9c800fd34ab7265a0e48c454ccbf4c9c61dfdf68f9a22000000010003f480000002632e314a0000002fbf9bd9c800";

#[test]
fn it_can_deserialize_a_vesting_contract() {
    let bytes: Vec<u8> = hex::decode(CONTRACT).unwrap();
    let contract: VestingContract = Deserialize::deserialize(&mut &bytes[..]).unwrap();
    assert_eq!(contract.balance, Coin::from(52500000000000));
    assert_eq!(contract.owner, Address::from("fd34ab7265a0e48c454ccbf4c9c61dfdf68f9a22"));
    assert_eq!(contract.vesting_start, 1);
    assert_eq!(contract.vesting_step_amount, Coin::from(2625000000000));
    assert_eq!(contract.vesting_step_blocks, 259200);
    assert_eq!(contract.vesting_total_amount, Coin::from(52500000000000));
}

#[test]
fn it_can_serialize_a_vesting_contract() {
    let bytes: Vec<u8> = hex::decode(CONTRACT).unwrap();
    let contract: VestingContract = Deserialize::deserialize(&mut &bytes[..]).unwrap();
    let mut bytes2: Vec<u8> = Vec::with_capacity(contract.serialized_size());
    let size = contract.serialize(&mut bytes2).unwrap();
    assert_eq!(size, contract.serialized_size());
    assert_eq!(hex::encode(bytes2), CONTRACT);
}


#[test]
fn it_can_verify_creation_transaction() {
    let mut data: Vec<u8> = Vec::with_capacity(Address::SIZE + 4);
    let owner = Address::from([0u8; 20]);
    Serialize::serialize(&owner, &mut data);
    Serialize::serialize(&100u32, &mut data);

    let mut transaction = Transaction::new_contract_creation(
        vec![],
        owner.clone(),
        AccountType::Basic,
        AccountType::Vesting,
        Coin::from(100),
        Coin::from(0),
        0,
        NetworkId::Dummy,
    );

    // Invalid data
    assert_eq!(VestingContract::verify_incoming_transaction(&transaction), Err(TransactionError::InvalidData));
    transaction.data = data;

    // Invalid recipient
    assert_eq!(VestingContract::verify_incoming_transaction(&transaction), Err(TransactionError::InvalidForRecipient));
    transaction.recipient = transaction.contract_creation_address();

    // Valid
    assert_eq!(VestingContract::verify_incoming_transaction(&transaction), Ok(()));

    // Invalid transaction flags
    transaction.flags = TransactionFlags::empty();
    transaction.recipient = transaction.contract_creation_address();
    assert_eq!(VestingContract::verify_incoming_transaction(&transaction), Err(TransactionError::InvalidForRecipient));
    transaction.flags = TransactionFlags::CONTRACT_CREATION;

    // Valid
    let mut data: Vec<u8> = Vec::with_capacity(Address::SIZE + 16);
    let sender = Address::from([0u8; 20]);
    Serialize::serialize(&sender, &mut data);
    Serialize::serialize(&100u32, &mut data);
    Serialize::serialize(&100u32, &mut data);
    Serialize::serialize(&Coin::from(100), &mut data);
    transaction.data = data;
    transaction.recipient = transaction.contract_creation_address();
    assert_eq!(VestingContract::verify_incoming_transaction(&transaction), Ok(()));

    // Valid
    let mut data: Vec<u8> = Vec::with_capacity(Address::SIZE + 24);
    let sender = Address::from([0u8; 20]);
    Serialize::serialize(&sender, &mut data);
    Serialize::serialize(&100u32, &mut data);
    Serialize::serialize(&100u32, &mut data);
    Serialize::serialize(&Coin::from(100), &mut data);
    Serialize::serialize(&Coin::from(100), &mut data);
    transaction.data = data;
    transaction.recipient = transaction.contract_creation_address();
    assert_eq!(VestingContract::verify_incoming_transaction(&transaction), Ok(()));
}

#[test]
fn it_can_create_contract_from_transaction() {
    let mut data: Vec<u8> = Vec::with_capacity(Address::SIZE + 4);
    let owner = Address::from([0u8; 20]);
    Serialize::serialize(&owner, &mut data);
    Serialize::serialize(&1000u32, &mut data);

    let mut transaction = Transaction::new_contract_creation(
        data,
        owner.clone(),
        AccountType::Basic,
        AccountType::Vesting,
        Coin::from(100),
        Coin::from(0),
        0,
        NetworkId::Dummy,
    );
    match VestingContract::create(Coin::from(0), &transaction, 0) {
        Ok(contract) => {
            assert_eq!(contract.balance, Coin::from(100));
            assert_eq!(contract.owner, owner);
            assert_eq!(contract.vesting_start, 0);
            assert_eq!(contract.vesting_step_blocks, 1000);
            assert_eq!(contract.vesting_step_amount, Coin::from(100));
            assert_eq!(contract.vesting_total_amount, Coin::from(100));
        }
        Err(_) => assert!(false)
    }

    let mut data: Vec<u8> = Vec::with_capacity(Address::SIZE + 16);
    let owner = Address::from([0u8; 20]);
    Serialize::serialize(&owner, &mut data);
    Serialize::serialize(&0u32, &mut data);
    Serialize::serialize(&100u32, &mut data);
    Serialize::serialize(&Coin::from(50), &mut data);
    transaction.data = data;
    transaction.recipient = transaction.contract_creation_address();
    match VestingContract::create(Coin::from(0), &transaction, 0) {
        Ok(contract) => {
            assert_eq!(contract.balance, Coin::from(100));
            assert_eq!(contract.owner, owner);
            assert_eq!(contract.vesting_start, 0);
            assert_eq!(contract.vesting_step_blocks, 100);
            assert_eq!(contract.vesting_step_amount, Coin::from(50));
            assert_eq!(contract.vesting_total_amount, Coin::from(100));
        }
        Err(_) => assert!(false)
    }

    let mut data: Vec<u8> = Vec::with_capacity(Address::SIZE + 24);
    let owner = Address::from([0u8; 20]);
    Serialize::serialize(&owner, &mut data);
    Serialize::serialize(&0u32, &mut data);
    Serialize::serialize(&100u32, &mut data);
    Serialize::serialize(&Coin::from(50), &mut data);
    Serialize::serialize(&Coin::from(150), &mut data);
    transaction.data = data;
    transaction.recipient = transaction.contract_creation_address();
    match VestingContract::create(Coin::from(0), &transaction, 0) {
        Ok(contract) => {
            assert_eq!(contract.balance, Coin::from(100));
            assert_eq!(contract.owner, owner);
            assert_eq!(contract.vesting_start, 0);
            assert_eq!(contract.vesting_step_blocks, 100);
            assert_eq!(contract.vesting_step_amount, Coin::from(50));
            assert_eq!(contract.vesting_total_amount, Coin::from(150));
        }
        Err(_) => assert!(false)
    }

    // Invalid data
    transaction.data = Vec::with_capacity(Address::SIZE + 2);
    transaction.recipient = transaction.contract_creation_address();
    assert_eq!(VestingContract::create(Coin::from(0), &transaction, 0), Err(AccountError::InvalidTransaction(TransactionError::InvalidData)))
}

#[test]
fn it_does_not_support_incoming_transactions() {
    let contract = VestingContract {
        balance: Coin::from(1000),
        owner: Address::from([1u8; 20]),
        vesting_start: 0,
        vesting_step_blocks: 100,
        vesting_step_amount: Coin::from(100),
        vesting_total_amount: Coin::from(1000),
    };

    let mut tx = Transaction::new_basic(Address::from([1u8; 20]), Address::from([2u8; 20]), Coin::from(1), Coin::from(1000), 1, NetworkId::Dummy);
    tx.recipient_type = AccountType::Vesting;

    assert_eq!(contract.with_incoming_transaction(&tx, 2), Err(AccountError::InvalidForRecipient));
    assert_eq!(contract.without_incoming_transaction(&tx, 2), Err(AccountError::InvalidForRecipient));
}

#[test]
fn it_can_verify_outgoing_transactions() {
    let key_pair = KeyPair::generate();
    let mut tx = Transaction::new_basic(Address::from([1u8; 20]), Address::from([2u8; 20]), Coin::from(1), Coin::from(1000), 1, NetworkId::Dummy);
    tx.sender_type = AccountType::Vesting;

    assert_eq!(VestingContract::verify_outgoing_transaction(&tx), Err(TransactionError::InvalidSerialization(SerializingError::IoError)));

    let signature = key_pair.sign(&tx.serialize_content()[..]);
    let signature_proof = SignatureProof::from(key_pair.public, signature);
    tx.proof = signature_proof.serialize_to_vec();

    assert_eq!(VestingContract::verify_outgoing_transaction(&tx), Ok(()));

    tx.proof[22] = tx.proof[22] % 250 + 1;

    assert_eq!(VestingContract::verify_outgoing_transaction(&tx), Err(TransactionError::InvalidProof));
}

#[test]
fn it_can_apply_and_revert_valid_transaction() {
    let key_pair = KeyPair::generate();
    let start_contract = VestingContract {
        balance: Coin::from(1000),
        owner: Address::from(&key_pair.public),
        vesting_start: 0,
        vesting_step_blocks: 100,
        vesting_step_amount: Coin::from(100),
        vesting_total_amount: Coin::from(1000),
    };

    let mut tx = Transaction::new_basic(Address::from([1u8; 20]), Address::from([2u8; 20]), Coin::from(200), Coin::from(0), 1, NetworkId::Dummy);
    tx.sender_type = AccountType::Vesting;

    let signature = key_pair.sign(&tx.serialize_content()[..]);
    let signature_proof = SignatureProof::from(key_pair.public, signature);
    tx.proof = signature_proof.serialize_to_vec();

    let mut contract = start_contract.with_outgoing_transaction(&tx, 200).unwrap();
    assert_eq!(contract.balance, Coin::from(800));
    contract = contract.without_outgoing_transaction(&tx, 1).unwrap();
    assert_eq!(contract, start_contract);

    let start_contract = VestingContract {
        balance: Coin::from(1000),
        owner: Address::from(&key_pair.public),
        vesting_start: 200,
        vesting_step_blocks: 0,
        vesting_step_amount: Coin::from(100),
        vesting_total_amount: Coin::from(1000),
    };

    let mut contract = start_contract.with_outgoing_transaction(&tx, 200).unwrap();
    assert_eq!(contract.balance, Coin::from(800));
    contract = contract.without_outgoing_transaction(&tx, 1).unwrap();
    assert_eq!(contract, start_contract);
}

#[test]
fn it_refuses_invalid_transaction() {
    let key_pair = KeyPair::generate();
    let key_pair_alt = KeyPair::generate();

    let start_contract = VestingContract {
        balance: Coin::from(1000),
        owner: Address::from(&key_pair.public),
        vesting_start: 0,
        vesting_step_blocks: 100,
        vesting_step_amount: Coin::from(100),
        vesting_total_amount: Coin::from(1000),
    };

    let mut tx = Transaction::new_basic(Address::from([1u8; 20]), Address::from([2u8; 20]), Coin::from(200), Coin::from(0), 1, NetworkId::Dummy);
    tx.sender_type = AccountType::Vesting;

    // Invalid signature
    let signature = key_pair_alt.sign(&tx.serialize_content()[..]);
    let signature_proof = SignatureProof::from(key_pair_alt.public, signature);
    tx.proof = signature_proof.serialize_to_vec();
    assert_eq!(start_contract.with_outgoing_transaction(&tx, 200), Err(AccountError::InvalidSignature));

    // Funds still vested
    let signature = key_pair.sign(&tx.serialize_content()[..]);
    let signature_proof = SignatureProof::from(key_pair.public, signature);
    tx.proof = signature_proof.serialize_to_vec();
    assert_eq!(start_contract.with_outgoing_transaction(&tx, 100), Err(AccountError::InsufficientFunds));
}
