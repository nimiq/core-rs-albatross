use beserial::Serialize;
use nimiq_genesis_builder::GenesisBuilder;
use nimiq_keys::{Address, KeyPair as SchnorrKeyPair, SecureGenerate};
use nimiq_primitives::coin::Coin;
use nimiq_primitives::networks::NetworkId;
use nimiq_transaction::{SignatureProof, Transaction};

#[derive(Clone)]
pub struct TestAccount {
    pub keypair: SchnorrKeyPair,
    pub address: Address,
}

#[derive(Clone)]
pub struct TestTransaction {
    pub fee: u64,
    pub value: u64,
    pub sender: TestAccount,
    pub recipient: TestAccount,
}

pub fn generate_accounts(
    balances: Vec<u64>,
    genesis_builder: &mut GenesisBuilder,
    add_to_genesis: bool,
) -> Vec<TestAccount> {
    let mut mempool_accounts = vec![];

    for balance in balances {
        // Generate the txns_sender and txns_rec vectors to later generate transactions
        let keypair = SchnorrKeyPair::generate_default_csprng();
        let address = Address::from(&keypair.public);
        let mempool_account = TestAccount { keypair, address };
        mempool_accounts.push(mempool_account);

        if add_to_genesis {
            // Add accounts to the genesis builder
            genesis_builder.with_basic_account(address, Coin::from_u64_unchecked(balance));
        }
    }
    mempool_accounts
}

pub fn generate_transactions(
    mempool_transactions: Vec<TestTransaction>,
    signature: bool,
) -> (Vec<Transaction>, usize) {
    let mut txns_len = 0;
    let mut txns: Vec<Transaction> = vec![];

    for mempool_transaction in mempool_transactions {
        // Generate transactions
        let mut txn = Transaction::new_basic(
            mempool_transaction.sender.address,
            mempool_transaction.recipient.address,
            Coin::from_u64_unchecked(mempool_transaction.value),
            Coin::from_u64_unchecked(mempool_transaction.fee),
            1,
            NetworkId::UnitAlbatross,
        );

        if signature {
            let signature_proof = SignatureProof::from(
                mempool_transaction.sender.keypair.public,
                mempool_transaction
                    .sender
                    .keypair
                    .sign(&txn.serialize_content()),
            );

            txn.proof = signature_proof.serialize_to_vec();
        }
        txns.push(txn.clone());
        //Adjust for executed txns len
        txns_len += 1 + txn.serialized_size();
    }
    (txns, txns_len)
}
