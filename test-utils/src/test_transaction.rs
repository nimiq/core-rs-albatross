use nimiq_genesis_builder::GenesisBuilder;
use nimiq_keys::{Address, KeyPair as SchnorrKeyPair, SecureGenerate};
use nimiq_primitives::{coin::Coin, networks::NetworkId, policy::Policy};
use nimiq_serde::Serialize;
use nimiq_transaction::{SignatureProof, Transaction};
use rand::{CryptoRng, Rng};

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

pub fn generate_accounts<R: Rng + CryptoRng>(
    balances: Vec<u64>,
    genesis_builder: &mut GenesisBuilder,
    add_to_genesis: bool,
    rng: &mut R,
) -> Vec<TestAccount> {
    let mut mempool_accounts = vec![];

    for balance in balances {
        // Generate the txns_sender and txns_rec vectors to later generate transactions
        let keypair = SchnorrKeyPair::generate(rng);
        let address = Address::from(&keypair.public);
        let mempool_account = TestAccount {
            keypair,
            address: address.clone(),
        };
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
            mempool_transaction.sender.address.clone(),
            mempool_transaction.recipient.address.clone(),
            Coin::from_u64_unchecked(mempool_transaction.value),
            Coin::from_u64_unchecked(mempool_transaction.fee),
            1 + Policy::genesis_block_number(),
            NetworkId::UnitAlbatross,
        );

        if signature {
            let signature_proof = SignatureProof::from_ed25519(
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
