use beserial::{Deserialize, Serialize};
use consensus::base::account::PrunedAccount;
use consensus::base::primitive::Address;
use consensus::base::primitive::hash::{Hash, HashOutput, SerializeContent};
use consensus::base::transaction::Transaction;
use consensus::networks::NetworkId;
use std::{cmp::Ordering, io};
use utils::merkle;

#[derive(Default, Clone, PartialEq, PartialOrd, Eq, Ord, Debug, Serialize, Deserialize)]
pub struct BlockBody {
    pub miner: Address,
    #[beserial(len_type(u8))]
    pub extra_data: Vec<u8>,
    #[beserial(len_type(u16))]
    pub transactions: Vec<Transaction>,
    #[beserial(len_type(u16))]
    pub pruned_accounts: Vec<PrunedAccount>,
}

impl SerializeContent for BlockBody {
    fn serialize_content<W: io::Write>(&self, writer: &mut W) -> io::Result<usize> { self.serialize(writer) }
}

impl Hash for BlockBody {
    fn hash<H: HashOutput>(&self) -> H {
        let mut vec: Vec<H> = Vec::with_capacity(2 + self.transactions.len() + self.pruned_accounts.len());
        vec.push(self.miner.hash());
        vec.push(self.extra_data.hash());
        for t in &self.transactions {
            vec.push(t.hash());
        }
        for p in &self.pruned_accounts {
            vec.push(p.hash());
        }
        return merkle::compute_root_from_hashes::<H>(&vec);
    }
}

#[allow(unreachable_code)]
impl BlockBody {
    pub(super) fn verify(&self, network_id: NetworkId) -> bool {
        let mut previous_tx: Option<&Transaction> = None;
        for tx in &self.transactions {
            // Ensure transactions are ordered and unique.
            if let Some(previous) = previous_tx {
                match previous.cmp_block_order(tx) {
                    Ordering::Equal => {
                        warn!("Invalid block - duplicate transaction");
                        return false;
                    }
                    Ordering::Greater => {
                        warn!("Invalid block - transactions not ordered");
                        return false;
                    }
                    _ => (),
                }
            }
            previous_tx = Some(tx);

            // Check that all transactions are valid.
            if !tx.verify() {
                warn!("Invalid block - invalid transaction");
                return false;
            }

            // Check that all transactions belong to this network
            if tx.network_id != network_id {
                return false;
            }
        }

        for acc in &self.pruned_accounts {
            // Ensure pruned accounts are ordered and unique.
            unimplemented!();

            // Check that pruned accounts are actually supposed to be pruned
            unimplemented!();
        }

        // Everything checks out.
        return true;
    }
}
