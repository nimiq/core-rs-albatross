use hash::Blake2bHash;
use keys::Address;
use primitives::transaction::TransactionReceipt;
use database::ReadTransaction;

use crate::blockchain::Blockchain;
use crate::transaction_store::TransactionInfo;

impl From<TransactionInfo> for TransactionReceipt {
    fn from(info: TransactionInfo) -> Self {
        TransactionReceipt {
            transaction_hash: info.transaction_hash,
            block_hash: info.block_hash,
            block_height: info.block_height,
        }
    }
}

impl<'env> Blockchain<'env> {
    pub fn get_transaction_receipts_by_address(&self, address: &Address, sender_limit: usize, recipient_limit: usize) -> Vec<TransactionReceipt> {
        let mut receipts;

        let txn = ReadTransaction::new(self.env);
        receipts = self.transaction_store.get_by_sender(address, sender_limit, Some(&txn));
        receipts.extend(self.transaction_store.get_by_recipient(address, recipient_limit, Some(&txn)));

        receipts.drain(..).map(|info| TransactionReceipt::from(info)).collect()
    }

    pub fn get_transaction_info_by_hash(&self, transaction_hash: &Blake2bHash) -> Option<TransactionInfo> {
        self.transaction_store.get_by_hash(transaction_hash, None)
    }
}