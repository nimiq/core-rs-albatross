use std::time::Duration;

use nimiq_blockchain::{
    history_store_proxy::UnmergedHistoryStoreProxy, interface::HistoryInterface, HistoryStore,
};
use nimiq_database::{
    mdbx::MdbxDatabase,
    traits::{Database, WriteTransaction},
};
use nimiq_hash::{Blake2bHash, Hash};
use nimiq_keys::Address;
use nimiq_primitives::{account::AccountType, coin::Coin, networks::NetworkId};
use nimiq_rpc::{
    primitives::{
        TransactionDetails2 as PoWTransaction, TransactionSequence as PoWTransactionSequence,
    },
    Client,
};
use nimiq_transaction::{
    historic_transaction::HistoricTransaction, ExecutedTransaction, Transaction, TransactionFlags,
};
use tokio::{
    sync::{mpsc, watch},
    time::sleep,
};

use crate::{async_retryer, types::HistoryError};

fn from_pow_network_id(pow_network_id: u8) -> Result<NetworkId, HistoryError> {
    match pow_network_id {
        1u8 => Ok(NetworkId::Test),
        42u8 => Ok(NetworkId::Main),
        _ => Err(HistoryError::InvalidValue),
    }
}

fn get_account_type(pow_account_type: &u8) -> Result<AccountType, HistoryError> {
    match pow_account_type {
        0u8 => Ok(AccountType::Basic),
        1u8 => Ok(AccountType::Vesting),
        2u8 => Ok(AccountType::HTLC),
        _ => Err(HistoryError::InvalidValue),
    }
}

fn from_pow_transaction(pow_transaction: &PoWTransaction) -> Result<Transaction, HistoryError> {
    let sender = Address::from_user_friendly_address(&pow_transaction.from_address)?;
    let sender_type = get_account_type(&pow_transaction.from_type)?;
    let recipient = Address::from_user_friendly_address(&pow_transaction.to_address)?;
    let recipient_type = get_account_type(&pow_transaction.to_type)?;
    let value = Coin::try_from(pow_transaction.value)?;
    let fee = Coin::try_from(pow_transaction.fee)?;
    let data = if let Some(data) = &pow_transaction.data {
        hex::decode(data)?
    } else {
        [].to_vec()
    };

    let validity_start_height = pow_transaction.validity_start_height;
    let network_id = from_pow_network_id(pow_transaction.network_id)?;
    let mut tx = Transaction::new_extended(
        sender,
        sender_type,
        [].to_vec(),
        recipient,
        recipient_type,
        data,
        value,
        fee,
        validity_start_height,
        network_id,
    );
    if let Some(proof) = &pow_transaction.proof {
        tx.proof = hex::decode(proof)?;
    }
    tx.flags = TransactionFlags::try_from(pow_transaction.flags)
        .map_err(|_| HistoryError::InvalidValue)?;
    Ok(tx)
}

/// Task that is responsible for migrating the PoW history into a PoS history up until an instructed block height.
/// It migrates the history up to the `candidate_block` received in `rx_candidate_block` if the head of the PoW chain
/// is greater than `candidate_block + block_confirmations` and if not waits for this to happen.
/// Note that this waiting process is done per block such that the migration can be triggered per confirmed block.
pub async fn migrate_history(
    mut rx_candidate_block: mpsc::Receiver<u32>,
    tx_migration_completed: watch::Sender<u32>,
    env: MdbxDatabase,
    network_id: NetworkId,
    pow_client: Client,
    block_confirmations: u32,
    index_history: bool,
) {
    let mut history_store_height = get_history_store_height(env.clone(), network_id).await;
    let history_store = UnmergedHistoryStoreProxy::new(env.clone(), index_history, network_id);
    let mut pow_head_height = async_retryer(|| pow_client.block_number()).await.unwrap();

    while let Some(candidate_block) = rx_candidate_block.recv().await {
        // Only migrate the part of the PoW history which we haven't processed yet
        if candidate_block <= history_store_height {
            tx_migration_completed.send(candidate_block).unwrap();
            continue;
        }

        log::info!(
            history_store_height,
            candidate_block,
            "Start migrating PoW history up to the next candidate block"
        );

        // Get transactions of each block and add them to the PoS history store
        // Migrate up to the candidate_block (exclusive) as this block number eventually becomes
        // the block number for the genesis block on the PoS chain.
        for block_height in (history_store_height + 1)..candidate_block {
            // Check if we are closing in on the PoW head block.
            // If this is true, we may need to take some extra time in order to make sure
            // that blocks leading up to the head block are confirmed before we
            // can safely add them to the PoS history store.
            if pow_head_height - block_height < block_confirmations {
                loop {
                    log::info!(
                        block = block_height,
                        "Waiting for block to be confirmed before it can be migrated"
                    );

                    sleep(Duration::from_secs(60)).await;
                    pow_head_height = async_retryer(|| pow_client.block_number()).await.unwrap();
                    // Check if the block has been confirmed.
                    if pow_head_height >= block_height + block_confirmations {
                        break;
                    }
                }
            }

            let block = async_retryer(|| pow_client.get_block_by_number(block_height, false))
                .await
                .unwrap();

            if block_height % 100 == 0 {
                log::info!(block_number = %block.number, target = %candidate_block, "Migrated new PoW history chunk");
            }

            // Get all transactions for this block height
            let mut transactions = vec![];
            let mut network_id = NetworkId::Main;
            match block.transactions {
                PoWTransactionSequence::BlockHashes(hashes) => {
                    if hashes.is_empty() {
                        // Mark that we've migrated up until this block.
                        history_store_height = block_height;
                        tx_migration_completed.send(block_height).unwrap();
                        continue;
                    }

                    for hash in hashes {
                        log::trace!(hash, "Processing transaction");
                        let pow_transaction =
                            async_retryer(|| pow_client.get_transaction_by_hash_2(&hash))
                                .await
                                .unwrap();
                        let pos_transaction = from_pow_transaction(&pow_transaction).unwrap();
                        network_id = pos_transaction.network_id;

                        assert_eq!(
                            pow_transaction.hash,
                            pos_transaction.hash::<Blake2bHash>().to_hex()
                        );
                        transactions.push(ExecutedTransaction::Ok(pos_transaction));
                    }

                    // Mark that we've migrated up until this block.
                    history_store_height = block_height;
                    tx_migration_completed.send(block_height).unwrap();
                }
                PoWTransactionSequence::Transactions(_) => panic!("Unexpected transaction type"),
            }

            // Add transactions to the history store
            let mut txn = env.write_transaction();
            history_store.add_to_history_for_epoch(
                &mut txn,
                0,
                block_height,
                &HistoricTransaction::from(
                    network_id,
                    block_height,
                    block.timestamp.into(),
                    transactions,
                    vec![],
                    vec![],
                ),
            );
            txn.commit();
        }

        log::info!(
            candidate_block,
            "Finished migrating PoW history up to the candidate block"
        );
        tx_migration_completed.send(candidate_block).unwrap();
    }
}

/// Get the PoS genesis history root by getting all of the transactions from the
/// PoW chain and building a single history tree.
pub async fn get_history_root(
    env: MdbxDatabase,
    network_id: NetworkId,
) -> Result<Blake2bHash, HistoryError> {
    HistoryStore::new(env.clone(), network_id)
        .get_history_tree_root(0, None)
        .ok_or(HistoryError::HistoryRootError)
}

/// Get the current block height of the PoS history store
pub async fn get_history_store_height(env: MdbxDatabase, network_id: NetworkId) -> u32 {
    HistoryStore::new(env.clone(), network_id)
        .get_last_leaf_block_number(None)
        .unwrap_or(1)
}

#[cfg(test)]
mod test {

    use super::*;

    static TRANSACTIONS: &str = r#"
[
    {
    "hash": "b216c4d1b655ebc918fcd25212f1f5abb1ed82a45a2114ee7109f92dba955f5c",
    "blockHash": "cbca3812447d51983a55beb2d16464b45dba19db29abdbbb7b9be8cead4a66a2",
    "blockNumber": 2815085,
    "timestamp": 1693405155,
    "confirmations": 11,
    "from": "fdcc85a8e604f23c7d2cd3c13a5d18dc566a6e7c",
    "fromAddress": "NQ02 YP68 BA76 0KR3 QY9C SF0K LP8Q THB6 LTKU",
    "fromType": 0,
    "to": "8b7566a432132996ef9d877039d027dc6e3dc4ce",
    "toAddress": "NQ57 HDSN D91J 2CLR DTUV GVQ3 KL17 THP3 TH6E",
    "toType": 0,
    "value": 10000,
    "fee": 138,
    "data": null,
    "proof": "ccbc66e792e3a8db41f0e561b9686ba6ce8dc1e84552c98bb254517d74f74d85002001864777677338cb6d2388df11dfbe3c03fa134235f35ccbb41b284a93d90dcbae1eca6baa0bd59d2ba954f08488887d07300c482e20f947a25752f2584c0f",
    "flags": 0,
    "validityStartHeight": 2815084,
    "networkId": 42
  },
  {
    "hash": "264cc24b364f286328459ba4a3a1b9982536e8dd2bfbf05bfc37265166f09aa3",
    "blockHash": "cbca3812447d51983a55beb2d16464b45dba19db29abdbbb7b9be8cead4a66a2",
    "blockNumber": 2815085,
    "timestamp": 1693405155,
    "confirmations": 11,
    "from": "ef81827f08614435a1592514140cc1ccbedc94e8",
    "fromAddress": "NQ55 VX0Q 4YQ8 C523 B8AR 4LA1 8361 RJYD R578",
    "fromType": 0,
    "to": "8de324671c791153e874cbccd9754cb0a44d2147",
    "toAddress": "NQ68 HPHJ 8RQU F48M 7S3L RF6D JVAC N2J4 S8A7",
    "toType": 0,
    "value": 34315526,
    "fee": 372,
    "data": null,
    "proof": "6b9a79b3aeb2d28b16632c888053c7156ca01b105b9e23ee09cbae2279f769390024b73d68c0e46d6b281bf20030734ee5f7110ce066ca368727bb0f678554d8d776db75061af1d0c55e30c7862998816cf0721875276e9240df53a43bf13ca009",
    "flags": 0,
    "validityStartHeight": 2815084,
    "networkId": 42
  },
  {
    "hash": "49d7a7b4b4a468974a9a870f64e6b0875f3693f180b8bd92990a87a7935e4369",
    "blockHash": "cbca3812447d51983a55beb2d16464b45dba19db29abdbbb7b9be8cead4a66a2",
    "blockNumber": 2815085,
    "timestamp": 1693405155,
    "confirmations": 11,
    "from": "bd8fb2a87312f4cf9ecb9646b06150757132b6d6",
    "fromAddress": "NQ08 PN7T 5A3K 2BSC Y7NB JR3B 0QAG EMQK 5DNN",
    "fromType": 0,
    "to": "8de324671c791153e874cbccd9754cb0a44d2147",
    "toAddress": "NQ68 HPHJ 8RQU F48M 7S3L RF6D JVAC N2J4 S8A7",
    "toType": 0,
    "value": 25729032,
    "fee": 319,
    "data": null,
    "proof": "cd8bd16ed7001f5cc3b5dc06375b61c5db32ef030e1ad371a48c0e4401a7f5c500e4623b75ce2923bd03f08a2bffb90b7964dddaafcd527c5b31d5ae93750a5ec575eab3a5335d6675d44c15e479db00f117f4e4f0b5f52a2189bd610b58cf870c",
    "flags": 0,
    "validityStartHeight": 2815084,
    "networkId": 42
  },
  {
    "hash": "3e2d9a86b648e0d33c5598311cc272c9e7216b1e989837912f80e661d7688735",
    "blockHash": "016fc6b7c9426beb7022e55b7388c36f0bfb7ef472e3297797fd2bf29234194d",
    "blockNumber": 2815086,
    "timestamp": 1693405211,
    "confirmations": 10,
    "from": "3367c64b81c1a65b3b1f02a20ce6d4f32d0aa8bf",
    "fromAddress": "NQ64 6DKU CJU1 Q6K5 NEQY 0AH0 RRNL XCNG MA5Y",
    "fromType": 0,
    "to": "0213056394b8f159b8c44415ea43d09c04afc5c8",
    "toAddress": "NQ22 089G AQUL P3QM KE64 8GAX LGXG KG2A YHE8",
    "toType": 0,
    "value": 88356098,
    "fee": 138,
    "data": null,
    "proof": "99427ba174a1fd0b3da910f57c9ba096acea4ebaa9b68fba63552aaf198ffc2700d1ba314c0bd25d205f9f60ccb9d5e90d9b40e62035ed37ad10ffa0283688900e615f15f9497fe9e3074915009ad9c53440376a32f1ee060ae46371401618580c",
    "flags": 0,
    "validityStartHeight": 2815084,
    "networkId": 42
  },
  {
    "hash": "889473ca3fd18b54e798ffa00fce94ecbcbeb615c8fe95312f654a6f93154443",
    "blockHash": "016fc6b7c9426beb7022e55b7388c36f0bfb7ef472e3297797fd2bf29234194d",
    "blockNumber": 2815086,
    "timestamp": 1693405211,
    "confirmations": 10,
    "from": "0b165dcb3b0af7a7e282bbf943a62ce93e0992b4",
    "fromAddress": "NQ18 1CB5 TJRT 1BTS FQL2 PFUL 79HC V4Y0 K4ML",
    "fromType": 0,
    "to": "0213056394b8f159b8c44415ea43d09c04afc5c8",
    "toAddress": "NQ22 089G AQUL P3QM KE64 8GAX LGXG KG2A YHE8",
    "toType": 0,
    "value": 81905456,
    "fee": 138,
    "data": null,
    "proof": "76bfb920de3027c9046cf99d793f39a10b7af636361443d0ee0fbc70d958f3d900189301500785ac96429d86b8e686e0515ee729bedccd4ea774fd0ebdc44bba6a0f9d6ad0a634cdfc56af26e57937d8a09446805ff2f51c9d82868e191a34f102",
    "flags": 0,
    "validityStartHeight": 2815084,
    "networkId": 42
  },
  {
    "hash": "59feba8c383ce7198b8870a5f9288df02528b6b17e3a373efe6a20235be43513",
    "blockHash": "016fc6b7c9426beb7022e55b7388c36f0bfb7ef472e3297797fd2bf29234194d",
    "blockNumber": 2815086,
    "timestamp": 1693405211,
    "confirmations": 10,
    "from": "256f74e41aa4f4688f17239bda1f3ce4d4402101",
    "fromAddress": "NQ50 4MPP 9R0S LKS6 H3QP 4EDV L7RU UKA4 0881",
    "fromType": 0,
    "to": "0213056394b8f159b8c44415ea43d09c04afc5c8",
    "toAddress": "NQ22 089G AQUL P3QM KE64 8GAX LGXG KG2A YHE8",
    "toType": 0,
    "value": 10920938,
    "fee": 138,
    "data": null,
    "proof": "2bfb6cf970a1fc9998d24952110f073a41fc84ddb8dbf2af97fcedb302b1dae400eb3a3f4a47cfd2498b783fb8b389edb44032fbea5d8f36729cec0b2ff1f56e9fb51e79cd3b4e863ba33be8d1f5258cde33a8a395766522e5f0c83b27344ca406",
    "flags": 0,
    "validityStartHeight": 2815084,
    "networkId": 42
  },
  {
    "hash": "df1a7b5fc4db53f316908e5e8ce3276f7ea8ff0d5647aa325ebd644519fb835e",
    "blockHash": "016fc6b7c9426beb7022e55b7388c36f0bfb7ef472e3297797fd2bf29234194d",
    "blockNumber": 2815086,
    "timestamp": 1693405211,
    "confirmations": 10,
    "from": "84ea97d004bab9db3a94a73dfc07810ce9f3f839",
    "fromAddress": "NQ81 GKM9 FL04 PAUV NELL LUXY Q1U1 1KLY 7X1R",
    "fromType": 0,
    "to": "8de324671c791153e874cbccd9754cb0a44d2147",
    "toAddress": "NQ68 HPHJ 8RQU F48M 7S3L RF6D JVAC N2J4 S8A7",
    "toType": 0,
    "value": 122785,
    "fee": 334,
    "data": null,
    "proof": "e90b9259ce6d9dfeada33a93bffc5cf19c19a01737f412d87648da03a71403ec000c5d4cd2d597092ded8a31cf6799f798265511bac256e7c7c1366e30c3d88094397c5771717918941be8ddf7cfd8ba13fe4d73824e7f68266808b606bc610108",
    "flags": 0,
    "validityStartHeight": 2815085,
    "networkId": 42
  },
  {
    "hash": "2c5b87aaa8c42307e631fffdb427cb3ae84ba3dbc91132be9a41b4864073c253",
    "blockHash": "477116d7f231a43fd18d3c9d447acc19188c09508c17a650fb997a2776500568",
    "blockNumber": 2815087,
    "timestamp": 1693405218,
    "confirmations": 9,
    "from": "45d341fead863fb0ed4a43410ab0222c5780e10a",
    "fromAddress": "NQ20 8P9L 3YMD GQYT 1TAA 8D0G MC12 5HBQ 1Q8A",
    "fromType": 0,
    "to": "9f410b4fc3132cbdbf65bb521ad322aecb5794a4",
    "toAddress": "NQ64 KV0G NKX3 2CNB TFT5 PD91 MLR2 MT5M F554",
    "toType": 0,
    "value": 31452,
    "fee": 214,
    "data": "4e696d69712053756e7365742043796265727370616365205061796f75742e2054533a31363933343035323131363236",
    "proof": "5868ef5d6819ec4641d6bb1aeb4fa18f1b940921a9f588ca2ae504cb4b8a14d700fbeccaa99bdc159dc534b919f04c1a38acc46f6e93d1e7ff9e47bf18d62a71688b79917fc195e40c1d2718683b1db98e263ca1a097c61cc746626bd63678c002",
    "flags": 0,
    "validityStartHeight": 2815085,
    "networkId": 42
  },
  {
    "hash": "96706e7446a07e9a5809c0689dab92b03c0ee73350642237807c752f26dee670",
    "blockHash": "94dacb7e12b46cba35fe859aa2c0951d6c6b3338d5ea3607457845961713fd39",
    "blockNumber": 2815089,
    "timestamp": 1693405265,
    "confirmations": 7,
    "from": "fdcc85a8e604f23c7d2cd3c13a5d18dc566a6e7c",
    "fromAddress": "NQ02 YP68 BA76 0KR3 QY9C SF0K LP8Q THB6 LTKU",
    "fromType": 0,
    "to": "0808c48c5c8b0d49dd2cb9364d57d0fa756aa238",
    "toAddress": "NQ57 104C 932U HC6L KP9C P4T4 SMXG Y9SN M8HQ",
    "toType": 0,
    "value": 10000,
    "fee": 138,
    "data": null,
    "proof": "ccbc66e792e3a8db41f0e561b9686ba6ce8dc1e84552c98bb254517d74f74d85005d32b69f31a7761de382041825ddb545542afc6e6e6862ee233d76590461a8039be77a0ede05217d2149edbb600cb7c47cd32872def385ce4df1711b495f6201",
    "flags": 0,
    "validityStartHeight": 2815088,
    "networkId": 42
  },
  {
    "hash": "a726b08f3edf7b636500ea66c3a469278156c132fef95851ccedb3be1087cebf",
    "blockHash": "a10f8774291d3010fe89b0872de97daf3d1da135ab74ad03dc42c24793aed4ce",
    "blockNumber": 2815091,
    "timestamp": 1693405305,
    "confirmations": 5,
    "from": "fdcc85a8e604f23c7d2cd3c13a5d18dc566a6e7c",
    "fromAddress": "NQ02 YP68 BA76 0KR3 QY9C SF0K LP8Q THB6 LTKU",
    "fromType": 0,
    "to": "ebda89ed462d3911b350101aeb12694202298f98",
    "toAddress": "NQ66 VFD8 KTA6 5LUH 3CSG 20DE N4K9 8812 K3UQ",
    "toType": 0,
    "value": 10000,
    "fee": 138,
    "data": null,
    "proof": "ccbc66e792e3a8db41f0e561b9686ba6ce8dc1e84552c98bb254517d74f74d8500f7688ac98a1f35e13dc39cd3cc9b4a3595f715bfb5a43adb244b3f58bfe4f4af938a17a9bccd17831a4c3e8e9e802f56ff20d74954b4422e8340346c8202b005",
    "flags": 0,
    "validityStartHeight": 2815089,
    "networkId": 42
  },
  {
    "hash": "e48752180c4ff77910b3f4b402043ebb8cad4a5766d64e6f81c7a4eabe9cc61d",
    "blockHash": "72f0b47e92935bf9b883072ba04a9abd6c94a51d827e727af7fc7fd80c00f2fd",
    "blockNumber": 2815092,
    "timestamp": 1693405344,
    "confirmations": 4,
    "from": "45d341fead863fb0ed4a43410ab0222c5780e10a",
    "fromAddress": "NQ20 8P9L 3YMD GQYT 1TAA 8D0G MC12 5HBQ 1Q8A",
    "fromType": 0,
    "to": "4884d8bab708c95c3b63cce127300d07897001e2",
    "toAddress": "NQ12 922D HEMP 134M QET3 RKGJ EC0D 0X4P 00F2",
    "toType": 0,
    "value": 121453,
    "fee": 214,
    "data": "4e696d69712053756e7365742043796265727370616365205061796f75742e2054533a31363933343035333132303239",
    "proof": "5868ef5d6819ec4641d6bb1aeb4fa18f1b940921a9f588ca2ae504cb4b8a14d7005852e59ee57537e295f6e5f9928b3f5d0b47ed5d6c804316a85988f6dfc1109da452678cbaca70d6a1bfb523606bc1374595bfa0ea96c1c5e63e0d381f90a90f",
    "flags": 0,
    "validityStartHeight": 2815090,
    "networkId": 42
  },
  {
    "hash": "35d5e6cf7d702c4d29d0e0f8a70643eddc9807d840e5ad049224603f68a73f0d",
    "blockHash": "72f0b47e92935bf9b883072ba04a9abd6c94a51d827e727af7fc7fd80c00f2fd",
    "blockNumber": 2815092,
    "timestamp": 1693405344,
    "confirmations": 4,
    "from": "0213056394b8f159b8c44415ea43d09c04afc5c8",
    "fromAddress": "NQ22 089G AQUL P3QM KE64 8GAX LGXG KG2A YHE8",
    "fromType": 0,
    "to": "83783f43b6d5a09f6a69522312e61c422968031c",
    "toAddress": "NQ11 GDU3 XGVN SNG9 XSK9 A8HH 5RGU 88LN G0QU",
    "toType": 0,
    "value": 60000000,
    "fee": 138,
    "data": null,
    "proof": "3ac7d5bfbe08b8021d3183b1e68fd86aa879dcbfd2b023725c09a75ff2e80cc300a21c134db436e3d9be35d2a57776547680001a6c20dc01bbc679d31d669968ccec66e792ac7b14fa1916771e592a0950ae2d1f661a0941583a1349df200b8c00",
    "flags": 0,
    "validityStartHeight": 2815090,
    "networkId": 42
  },
  {
    "hash": "a55dd3ab771adba54678144a2d7653907b719cc32d265e4266a2c904a06639e2",
    "blockHash": "72f0b47e92935bf9b883072ba04a9abd6c94a51d827e727af7fc7fd80c00f2fd",
    "blockNumber": 2815092,
    "timestamp": 1693405344,
    "confirmations": 4,
    "from": "45d341fead863fb0ed4a43410ab0222c5780e10a",
    "fromAddress": "NQ20 8P9L 3YMD GQYT 1TAA 8D0G MC12 5HBQ 1Q8A",
    "fromType": 0,
    "to": "e09a78023f09514f549ccdcc6b75f406c20a99d5",
    "toAddress": "NQ30 U2D7 G0HY 158L XM4U RP66 NVFL 0T10 M6EM",
    "toType": 0,
    "value": 91453,
    "fee": 214,
    "data": "4e696d69712053756e7365742043796265727370616365205061796f75742e2054533a31363933343035333036323733",
    "proof": "5868ef5d6819ec4641d6bb1aeb4fa18f1b940921a9f588ca2ae504cb4b8a14d70031ba7d93cfaa11af4c55f4f3e96c26678916d19c5baa60243693fb8afb89f492bcf85d084d72f4d7abce9cf238fad79994c399e06df5b336913a93c7618e6c02",
    "flags": 0,
    "validityStartHeight": 2815090,
    "networkId": 42
  },
  {
    "hash": "2cdb1360941a24eacba09d598b43e03a2009582703f9ca2171b9f0c699eda129",
    "blockHash": "f44fdc76055b2e03c9dbe3290555545ea24e6ee1594a2254bd89084d353a4b78",
    "blockNumber": 2815094,
    "timestamp": 1693405408,
    "confirmations": 2,
    "from": "79020efa4be4c0236e9047e3b2548ba18c4b5ea7",
    "fromAddress": "NQ21 F410 VXJB UK02 6TLG 8YHT 4M4B L664 NPM7",
    "fromType": 0,
    "to": "4b12c6250cbbdd04c2b5a6cb9ee872ed2be2ee57",
    "toAddress": "NQ42 9C9C C98C PFEG 9GMM LT5R VS3J VLMX 5TJP",
    "toType": 0,
    "value": 450000,
    "fee": 0,
    "data": null,
    "proof": "580c080fd25f60a13b37d2f2bf0a94090d0b8ffef741452c288f1d9d5167863f00662245562d0e9ed3ca996617ec4999f5381295510097f5c7906d279516a1926e5bcdf7d00d8a930ccb546461cca76735655cbc994b654d6dbfda8f5d6d47cf0a",
    "flags": 0,
    "validityStartHeight": 2815092,
    "networkId": 42
  }
]"#;

    #[test]
    fn can_parse_pow_transactions() {
        let pow_transactions: Vec<PoWTransaction> = serde_json::from_str(TRANSACTIONS).unwrap();
        for txn in pow_transactions {
            let pos_transaction = from_pow_transaction(&txn).unwrap();
            assert_eq!(txn.hash, pos_transaction.hash::<Blake2bHash>().to_hex())
        }
    }
}
