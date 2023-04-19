use nimiq_keys::Address;
use nimiq_primitives::{account::AccountType, networks::NetworkId, transaction::TransactionError};
use nimiq_transaction::{account::AccountTransactionVerification, Transaction};

#[test]
fn it_does_not_allow_creation() {
    let owner = Address::from([0u8; 20]);

    let transaction = Transaction::new_contract_creation(
        vec![],
        owner,
        AccountType::Basic,
        AccountType::Basic,
        100.try_into().unwrap(),
        0.try_into().unwrap(),
        0,
        NetworkId::Dummy,
    );

    assert_eq!(
        AccountType::verify_incoming_transaction(&transaction),
        Err(TransactionError::InvalidForRecipient)
    );
}

#[test]
fn it_does_not_allow_signalling() {
    let owner = Address::from([0u8; 20]);

    let transaction = Transaction::new_signaling(
        owner,
        AccountType::Basic,
        Address::from([1u8; 20]),
        AccountType::Basic,
        0.try_into().unwrap(),
        vec![],
        0,
        NetworkId::Dummy,
    );

    assert_eq!(
        AccountType::verify_incoming_transaction(&transaction),
        Err(TransactionError::ZeroValue)
    );
}
