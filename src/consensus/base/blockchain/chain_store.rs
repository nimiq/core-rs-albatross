use consensus::base::block::Block;
use consensus::base::blockchain::ChainData;
use consensus::base::primitive::hash::Blake2bHash;
use utils::db::{Database, Transaction, ReadTransaction, WriteTransaction, Environment};

#[derive(Debug)]
pub struct ChainStore<'env> {
    env: &'env Environment,
    chain_db: Database<'env>,
    block_db: Database<'env>
}

impl<'env> ChainStore<'env> {
    const CHAIN_DB_NAME: &'static str = "ChainData";
    const BLOCK_DB_NAME: &'static str = "Block";
    const HEAD_KEY: &'static str = "head";

    pub fn new(env: &'env Environment) -> Self {
        let chain_db = env.open_database(Self::CHAIN_DB_NAME.to_string());
        let block_db = env.open_database(Self::CHAIN_DB_NAME.to_string());
        return ChainStore { env, chain_db, block_db };
    }

    pub fn get_head(&self, txn_option: Option<&Transaction>) -> Option<Blake2bHash> {
        return match txn_option {
            Some(txn) => txn.get(&self.chain_db, ChainStore::HEAD_KEY),
            None => ReadTransaction::new(self.env).get(&self.chain_db, ChainStore::HEAD_KEY)
        };
    }

    pub fn set_head(&self, txn: &mut WriteTransaction, hash: &Blake2bHash) {
        txn.put(&self.chain_db, ChainStore::HEAD_KEY, hash);
    }

    pub fn get_chain_data(&self, hash: &Blake2bHash, include_body: bool, txn_option: Option<&Transaction>) -> Option<ChainData> {
        let read_txn: ReadTransaction;
        let txn = match txn_option {
            Some(txn) => txn,
            None => {
                read_txn = ReadTransaction::new(self.env);
                &read_txn
            }
        };

        let chain_data_opt = txn.get(&self.chain_db, hash);
        if chain_data_opt.is_none() {
            return None;
        }

        let mut chain_data = chain_data_opt.unwrap();
        if !include_body {
            return Some(chain_data);
        }

        let block_opt = txn.get(&self.block_db, hash);
        if block_opt.is_none() {
            warn!("Block body requested but not present");
            return Some(chain_data);
        }

        chain_data.head = block_opt.unwrap();
        return Some(chain_data);
    }

    pub fn put_chain_data(&self, txn: &mut WriteTransaction, hash: &Blake2bHash, chain_data: &ChainData, include_body: bool) {
        txn.put(&self.chain_db, hash, chain_data);

        if include_body && chain_data.head.body.is_some() {
            txn.put(&self.block_db, hash, &chain_data.head);
        }
    }

    pub fn get_chain_data_at(&self, block_height: u32) -> Option<ChainData> {
        unimplemented!();
    }

    pub fn get_block(&self, hash: &Blake2bHash) -> Option<Block> {
        unimplemented!();
    }

    pub fn get_block_at(&self, block_height: u32) -> Option<Block> {
        unimplemented!();
    }
}
